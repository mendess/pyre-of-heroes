use std::{collections::HashMap, io, thread::available_parallelism};

use futures::{stream::TryStreamExt, Stream};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    sync::{OnceCell, RwLock},
};
use tokio_stream::wrappers::LinesStream;

use crate::Card;

fn card_name_trimmer(mut s: &str) -> &str {
    s = s.trim();
    if s.chars().next().map(|c| c.is_ascii_digit()) == Some(true) {
        let mut i = s.chars();
        i.by_ref()
            .take_while(|c| c.is_ascii_digit())
            .for_each(|_| {});
        s = i.as_str().trim()
    };
    s
}

fn cmc_f32_to_u8(f: f32) -> Option<u8> {
    let lower = f as u16;
    let upper = lower + 1;
    if f > lower as f32 && f < upper as f32 {
        None
    } else {
        lower.try_into().ok()
    }
}

type Cache = HashMap<String, Card>;

static CACHE: OnceCell<RwLock<Cache>> = OnceCell::const_new();

const CACHE_PATH: &str = "cache.json";
const CACHE_PATH_TMP: &str = "cache.json.tmp";

async fn cache() -> io::Result<&'static RwLock<HashMap<String, Card>>> {
    CACHE
        .get_or_try_init(|| async {
            let buf = match tokio::fs::read(CACHE_PATH).await {
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Default::default()),
                r => r,
            }?;
            io::Result::Ok(RwLock::new(serde_json::from_slice(&buf)?))
        })
        .await
}

async fn find_in_cache(name: &str) -> io::Result<Option<Card>> {
    let cache = cache().await?;
    Ok(cache.read().await.get(name).cloned())
}

async fn store_in_cache(name: &str, card: &Card) -> io::Result<()> {
    let cache = cache().await?;
    let mut cache = cache.write().await;
    cache.insert(name.into(), card.clone());
    let mut file = File::create(CACHE_PATH_TMP).await?;
    file.write_all(&serde_json::to_vec::<Cache>(&*cache).unwrap())
        .await?;
    tokio::fs::rename(CACHE_PATH_TMP, CACHE_PATH).await
}

async fn fetch_card(name: &str) -> scryfall::Result<Card> {
    match find_in_cache(name).await {
        Ok(Some(card)) => return Ok(card),
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            eprintln!("failed to fetch from cache: {e:?}");
        }
        _ => {
            eprintln!("cache miss: {name}");
        }
    }
    let card = scryfall::Card::named_fuzzy(name).await?;
    let types = card
        .type_line
        .map(|t| t.split(' ').map(ToOwned::to_owned).collect())
        .unwrap_or_default();

    let cmc = cmc_f32_to_u8(
        card.cmc
            .unwrap_or_else(|| panic!("{} doens't have cmc", card.name)),
    )
    .unwrap_or_else(|| panic!("{} has a fractional cmc", card.name));
    let card = Card {
        cmc,
        name: card.name,
        types,
    };
    if let Err(e) = store_in_cache(name, &card).await {
        eprintln!("failed to store in cache: {e:?}");
    }
    Ok(card)
}

pub(super) async fn parse<'r, R: AsyncRead + 'r>(
    r: R,
) -> impl Stream<Item = scryfall::Result<Card>> + 'r {
    let reader = BufReader::new(r);
    LinesStream::new(reader.lines())
        .map_err(scryfall::Error::from)
        .map_ok(|line| async move {
            let mut card = fetch_card(card_name_trimmer(&line)).await?;
            Ok((card.types.iter().any(|t| t == "Creature")).then(|| {
                if let Some(dash) = card.types.iter().position(|s| s == "—") {
                    card.types.drain(..=dash).for_each(|_| {});
                }
                card
            }))
        })
        .try_buffer_unordered(available_parallelism().unwrap().get())
        .try_filter_map(|r| futures::future::ready(Ok(r)))
}
