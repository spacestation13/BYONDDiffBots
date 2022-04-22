use std::{collections::HashSet, path::Path, sync::RwLock};

use ahash::RandomState;
use anyhow::Result;
use dm::objtree::ObjectTree;
use dmm_tools::{dmi::Image, dmm, minimap, render_passes::RenderPass, IconCache};

use crate::{git_operations::*, github_types::*};
extern crate dreammaker as dm;

#[derive(Debug)]
pub struct BoundingBox {
    pub left: usize,
    pub bottom: usize,
    pub right: usize,
    pub top: usize,
}

pub fn get_diff_bounding_box(
    left_map: &dmm::Map,
    right_map: &dmm::Map,
    z_level: usize,
) -> BoundingBox {
    eprintln!("getting bounding box");
    use std::cmp::min;

    let left_dims = left_map.dim_xyz();
    let right_dims = right_map.dim_xyz();
    if left_dims != right_dims {
        println!("    different size: {:?} {:?}", left_dims, right_dims);
    }

    let mut rightmost = 0usize;
    let mut leftmost = left_dims.0;
    let mut topmost = 0usize;
    let mut bottommost = left_dims.1;

    for y in 0..min(left_dims.1, right_dims.1) {
        for x in 0..min(left_dims.0, right_dims.0) {
            let left_tile = &left_map.dictionary[&left_map.grid[(z_level, left_dims.1 - y - 1, x)]];
            let right_tile =
                &right_map.dictionary[&right_map.grid[(z_level, right_dims.1 - y - 1, x)]];
            if left_tile != right_tile {
                if x < leftmost {
                    leftmost = x;
                }
                if x > rightmost {
                    rightmost = x;
                }
                if y < bottommost {
                    bottommost = y;
                }
                if y > topmost {
                    topmost = y;
                }
            }
        }
    }

    if leftmost > rightmost {
        leftmost = 0;
        rightmost = 1;
        bottommost = 0; // create a small bounding box for now if there are no changes
        topmost = 1;
    }

    BoundingBox {
        left: leftmost,
        bottom: bottommost,
        right: rightmost,
        top: topmost,
    }
}

pub struct MapDiffs {
    pub bases: Vec<dmm::Map>,
    pub heads: Vec<dmm::Map>,
    pub bbs: Vec<BoundingBox>,
}

pub fn get_map_diffs(base: &Branch, head: &str, files: &Vec<&ModifiedFile>) -> Result<MapDiffs> {
    eprintln!("getting map diffs");

    let load_maps = || {
        files
            .iter()
            .map(|file| {
                dmm::Map::from_file(Path::new(&file.filename)).map_err(|e| anyhow::anyhow!(e))
            })
            .collect()
    };

    let lefts: Vec<dmm::Map> = with_checkout(&base.repo.name, &base.name, load_maps)?;
    let rights: Vec<dmm::Map> = with_checkout(&base.repo.name, &head, load_maps)?;

    let bbs = lefts
        .iter()
        .zip(rights.iter())
        .map(|(left, right)| (get_diff_bounding_box(left, right, 0)))
        .collect();

    Ok(MapDiffs {
        bases: lefts,
        heads: rights,
        bbs,
    })
}

#[derive(Default)]
pub struct Context {
    pub dm_context: dm::Context,
    pub objtree: ObjectTree,
    pub icon_cache: IconCache,
}

impl Context {
    pub fn objtree(&mut self, path: &Path) -> Result<()> {
        let environment = match dm::detect_environment(path, dm::DEFAULT_ENV) {
            Ok(Some(found)) => found,
            _ => dm::DEFAULT_ENV.into(),
        };
        eprintln!("parsing {}", environment.display());

        if let Some(parent) = environment.parent() {
            self.icon_cache.set_icons_root(&parent);
        }

        self.dm_context.autodetect_config(&environment);
        let pp = dm::preprocessor::Preprocessor::new(&self.dm_context, environment)?;
        let indents = dm::indents::IndentProcessor::new(&self.dm_context, pp);
        let parser = dm::parser::Parser::new(&self.dm_context, indents);
        self.objtree = parser.parse_object_tree();
        Ok(())
    }
}

pub fn render_map(
    objtree: &ObjectTree,
    icon_cache: &IconCache,
    map: &dmm::Map,
    bb: &BoundingBox,
    errors: &RwLock<HashSet<String, RandomState>>,
    render_passes: &Vec<Box<dyn RenderPass>>,
) -> Result<Image> {
    let bump = Default::default();
    let minimap_context = minimap::Context {
        objtree: objtree,
        map: &map,
        level: map.z_level(0),
        min: (bb.left, bb.bottom),
        max: (bb.right, bb.top),
        render_passes: &render_passes,
        errors: &errors,
        bump: &bump,
    };
    println!("Generating map");
    eprintln!("{:?}", bb);
    minimap::generate(minimap_context, icon_cache)
        .map_err(|_| anyhow::anyhow!("An error occured during map rendering"))
}
