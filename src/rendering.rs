use std::{cmp::min, collections::HashSet, path::Path, sync::RwLock};

extern crate dreammaker as dm;

use ahash::RandomState;
use anyhow::Result;
use dm::objtree::ObjectTree;
use dmm_tools::{dmi::Image, dmm, minimap, render_passes::RenderPass, IconCache};

use crate::{git_operations::*, github_types::*};

#[derive(Debug)]
pub struct BoundingBox {
    pub left: usize,
    pub bottom: usize,
    pub right: usize,
    pub top: usize,
}

pub fn get_diff_bounding_box(
    base_map: &dmm::Map,
    head_map: &dmm::Map,
    z_level: usize,
) -> BoundingBox {
    eprintln!("getting bounding box");

    let left_dims = base_map.dim_xyz();
    let right_dims = head_map.dim_xyz();
    if left_dims != right_dims {
        println!("    different size: {:?} {:?}", left_dims, right_dims);
    }

    let mut rightmost = 0usize;
    let mut leftmost = left_dims.0;
    let mut topmost = 0usize;
    let mut bottommost = left_dims.1;

    for y in 0..min(left_dims.1, right_dims.1) {
        for x in 0..min(left_dims.0, right_dims.0) {
            let left_tile = &base_map.dictionary[&base_map.grid[(z_level, left_dims.1 - y - 1, x)]];
            let right_tile =
                &head_map.dictionary[&head_map.grid[(z_level, right_dims.1 - y - 1, x)]];
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
    pub base_maps: Vec<dmm::Map>,
    pub head_maps: Vec<dmm::Map>,
    pub bounds: Vec<BoundingBox>,
}

pub fn get_map_diffs(base: &Branch, head: &str, files: &[&ModifiedFile]) -> Result<MapDiffs> {
    eprintln!("getting map diffs");

    let load_maps = || {
        files
            .iter()
            .map(|file| {
                dmm::Map::from_file(Path::new(&file.filename)).map_err(|e| anyhow::anyhow!(e))
            })
            .collect()
    };

    let base_maps: Vec<_> = with_checkout(&base.repo.name, &base.name, load_maps)?;
    let head_maps: Vec<_> = with_checkout(&base.repo.name, head, load_maps)?;

    let bounds = base_maps
        .iter()
        .zip(head_maps.iter())
        .map(|(base, head)| (get_diff_bounding_box(base, head, 0)))
        .collect();

    Ok(MapDiffs {
        base_maps,
        head_maps,
        bounds,
    })
}

#[derive(Default)]
pub struct Context {
    pub dm_context: dm::Context,
    pub obj_tree: ObjectTree,
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
            self.icon_cache.set_icons_root(parent);
        }

        self.dm_context.autodetect_config(&environment);
        let pp = dm::preprocessor::Preprocessor::new(&self.dm_context, environment)?;
        let indents = dm::indents::IndentProcessor::new(&self.dm_context, pp);
        let parser = dm::parser::Parser::new(&self.dm_context, indents);
        self.obj_tree = parser.parse_object_tree();
        Ok(())
    }
}

pub fn render_map(
    objtree: &ObjectTree,
    icon_cache: &IconCache,
    map: &dmm::Map,
    bounds: &BoundingBox,
    errors: &RwLock<HashSet<String, RandomState>>,
    render_passes: &[Box<dyn RenderPass>],
) -> Result<Image> {
    let bump = Default::default();
    let minimap_context = minimap::Context {
        objtree,
        map,
        level: map.z_level(0),
        min: (bounds.left, bounds.bottom),
        max: (bounds.right, bounds.top),
        render_passes,
        errors,
        bump: &bump,
    };
    println!("Generating map");
    eprintln!("{:?}", bounds);
    minimap::generate(minimap_context, icon_cache)
        .map_err(|_| anyhow::anyhow!("An error occured during map rendering"))
}
