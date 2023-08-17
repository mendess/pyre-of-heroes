use std::{collections::HashMap, fmt::Display, hash::Hash, io, marker::PhantomData, path::Path};

use petgraph::{algo::DfsSpace, prelude::NodeIndex, Graph};
use tokio::{
    fs::File,
    io::{AsyncWriteExt, BufWriter},
};

use crate::Card;

pub(super) struct Link<Edge> {
    edge: Edge,
    dir: LinkDirection,
}

enum LinkDirection {
    From,
    To,
}

pub(super) trait PodKind {
    type Edge: Display + Hash + Eq;
    fn check(new: &Card, existing: &Card) -> Option<Link<Self::Edge>>;
}

pub struct BirthingPod;

#[derive(Debug, Hash, PartialEq, Eq, Copy, Clone, PartialOrd, Ord)]
pub(crate) struct NoInfo;

impl Display for NoInfo {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(())
    }
}

impl PodKind for BirthingPod {
    type Edge = NoInfo;
    fn check(new: &Card, existing: &Card) -> Option<Link<Self::Edge>> {
        match (new.cmc as i16) - (existing.cmc as i16) {
            -1 => Some(Link {
                edge: NoInfo,
                dir: LinkDirection::To,
            }),
            1 => Some(Link {
                edge: NoInfo,
                dir: LinkDirection::From,
            }),
            _ => None,
        }
    }
}

pub struct PyreOfHeroes;

impl PodKind for PyreOfHeroes {
    type Edge = String;
    fn check(new: &Card, existing: &Card) -> Option<Link<Self::Edge>> {
        if let Some(ty) = new.types.iter().find(|t| existing.types.contains(t)) {
            BirthingPod::check(new, existing).map(|t| Link {
                edge: ty.clone(),
                dir: t.dir,
            })
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub(crate) struct PodGraph<K: PodKind> {
    g: Graph<Card, K::Edge>,
    _pod: PhantomData<K>,
}

impl<K: PodKind> PodGraph<K> {
    pub fn new() -> Self {
        Self {
            g: Default::default(),
            _pod: PhantomData,
        }
    }

    pub fn add_card(&mut self, c: Card) {
        let links = self
            .g
            .node_indices()
            .filter_map(|n| K::check(&c, &self.g[n]).map(|l| (n, l)))
            .collect::<Vec<_>>();
        let node = self.g.add_node(c);
        for (existing_node, link) in links {
            match link.dir {
                LinkDirection::From => self.g.add_edge(existing_node, node, link.edge),
                LinkDirection::To => self.g.add_edge(node, existing_node, link.edge),
            };
        }
    }

    fn nodes_that_can_reach(&self, name: &str) -> Vec<NodeIndex> {
        let Some(target) = self.g.node_indices().find(|n| self.g[*n].name.contains(name)) else {
            return Default::default();
        };
        let mut space = DfsSpace::new(&self.g);
        self.g
            .node_indices()
            .filter(|n| petgraph::algo::has_path_connecting(&self.g, *n, target, Some(&mut space)))
            .collect()
    }

    pub async fn to_img<P: AsRef<Path>>(
        &self,
        path: P,
        draw_path_to: Option<&str>,
    ) -> io::Result<()> {
        let highlight = draw_path_to.map(|name| self.nodes_that_can_reach(name));
        let mut file = BufWriter::new(File::create(path).await?);
        file.write_all(
            b"digraph {\n    node [colorscheme=spectral11]\nedge [colorscheme=dark28]\n",
        )
        .await?;
        let subgraphs = self
            .g
            .node_indices()
            .fold(HashMap::<_, Vec<_>>::new(), |mut acc, n| {
                acc.entry(self.g[n].cmc).or_default().push(n);
                acc
            });
        for (cmc, subgraph) in subgraphs {
            file.write_all(format!("    subgraph cluster_{cmc} {{\n").as_bytes())
                .await?;
            for n in subgraph {
                let buf = format!(
                    "        {} [ label = \"{}\" {style} {hi}]\n",
                    n.index(),
                    self.g[n].name,
                    style = match self.node_is_isolated(&n) {
                        true => "style=filled fillcolor=2",
                        false => "",
                    },
                    hi = match &highlight {
                        Some(highlight) if highlight.contains(&n) => "style=filled fillcolor=11",
                        _ => "",
                    }
                );
                file.write_all(buf.as_bytes()).await?;
            }
            file.write_all(format!("       label = \"{cmc}\"\n").as_bytes())
                .await?;
            file.write_all(b"   }\n").await?;
        }
        let mut link_color = HashMap::new();
        for e in self.g.edge_indices() {
            let (from, to) = self.g.edge_endpoints(e).unwrap();
            if let Some(highlight) = &highlight {
                if !(highlight.contains(&from) && highlight.contains(&to)) {
                    continue;
                }
            }
            let color_count = link_color.len();
            let color = link_color
                .entry(&self.g[e])
                .or_insert_with(|| color_count + 1);
            let buf = format!(
                "{} -> {} [ label = \"{}\" color={color} fontcolor={color}]\n",
                from.index(),
                to.index(),
                self.g[e],
            );
            file.write_all(buf.as_bytes()).await?;
        }
        file.write_all(b"}").await?;
        file.flush().await?;
        Ok(())
    }

    fn node_is_isolated(&self, index: &NodeIndex) -> bool {
        self.g.edge_indices().all(|e| {
            self.g
                .edge_endpoints(e)
                .map(|(from, to)| *index != from && *index != to)
                .unwrap_or_default()
        })
    }
}
