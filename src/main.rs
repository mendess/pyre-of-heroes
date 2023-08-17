mod decklist;
mod pyre_graph;

use std::{path::PathBuf, pin::Pin};

use clap::Parser;
use futures::{Stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use tokio::{fs::File, io::stdin};

#[derive(Parser)]
struct Args {
    file: Option<PathBuf>,
    #[arg(short = 't', long)]
    highlight: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Card {
    name: String,
    cmc: u8,
    types: Vec<String>,
}

#[tokio::main]
async fn main() -> scryfall::Result<()> {
    let args = Args::parse();
    let creatures = match args.file {
        Some(path) if path.as_os_str() != "-" => {
            decklist::parse(File::open(path).await?).await.boxed()
                as Pin<Box<dyn Stream<Item = scryfall::Result<Card>>>>
        }
        _ => decklist::parse(stdin()).await.boxed(),
    };
    let graph = creatures
        .try_fold(
            pyre_graph::PodGraph::<pyre_graph::BirthingPod>::new(),
            |mut g, c| async move {
                eprintln!("added {}", c.name);
                g.add_card(c);
                Ok(g)
            },
        )
        .await?;
    graph.to_img("graph.dot", args.highlight.as_deref()).await?;
    Ok(())
}
