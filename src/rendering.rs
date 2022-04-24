use std::{cell::Ref, cmp::min, collections::HashSet, path::Path, sync::RwLock};

extern crate dreammaker as dm;

use ahash::RandomState;
use anyhow::{Context, Result};
use dm::objtree::ObjectTree;
use dmm_tools::{dmi::Image, dmm, minimap, render_passes::RenderPass, IconCache};
use rayon::prelude::*;

use crate::github_types::*;

#[derive(Debug)]
pub struct BoundingBox {
    left: usize,
    bottom: usize,
    right: usize,
    top: usize,
}

impl BoundingBox {
    pub fn new(left: usize, bottom: usize, right: usize, top: usize) -> Self {
        Self {
            left,
            bottom,
            right,
            top,
        }
    }

    pub fn for_full_map(map: &dmm::Map) -> Self {
        let dims = map.dim_xyz();
        Self {
            left: 0,
            bottom: 0,
            right: dims.0 - 1,
            top: dims.1 - 1,
        }
    }
}

pub type RenderingErrors = RwLock<HashSet<String, RandomState>>;

pub fn get_diff_bounding_box(
    base_map: &dmm::Map,
    head_map: &dmm::Map,
    z_level: usize,
) -> BoundingBox {
    let left_dims = base_map.dim_xyz();
    let right_dims = head_map.dim_xyz();
    if left_dims != right_dims {
        println!(
            "Maps have different sizes: {:?} {:?}",
            left_dims, right_dims
        );
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

    BoundingBox::new(leftmost, bottommost, rightmost, topmost)
}

pub fn load_maps(files: &[&ModifiedFile]) -> Result<Vec<dmm::Map>> {
    files
        .iter()
        .map(|file| dmm::Map::from_file(Path::new(&file.filename)).map_err(|e| anyhow::anyhow!(e)))
        .collect()
}

pub fn get_map_diff_bounding_boxes(
    base_maps: &[dmm::Map],
    head_maps: &[dmm::Map],
) -> Vec<BoundingBox> {
    base_maps
        .iter()
        .zip(head_maps.iter())
        .map(|(base, head)| (get_diff_bounding_box(base, head, 0)))
        .collect()
}

pub struct RenderingContext {
    dm_context: dm::Context,
    obj_tree: ObjectTree,
    icon_cache: IconCache,
}

impl RenderingContext {
    pub fn config(&self) -> Ref<dm::config::Config> {
        self.dm_context.config()
    }
}

impl RenderingContext {
    pub fn new(path: &Path) -> Result<Self> {
        let dm_context = dm::Context::default();
        let mut icon_cache = IconCache::default();

        let environment = match dm::detect_environment(path, dm::DEFAULT_ENV) {
            Ok(Some(found)) => found,
            _ => dm::DEFAULT_ENV.into(),
        };

        eprintln!("Parsing {}", environment.display());

        if let Some(parent) = environment.parent() {
            icon_cache.set_icons_root(parent);
        }

        dm_context.autodetect_config(&environment);
        let pp = dm::preprocessor::Preprocessor::new(&dm_context, environment)
            .context("Creating preprocessor")?;
        let indents = dm::indents::IndentProcessor::new(&dm_context, pp);
        let parser = dm::parser::Parser::new(&dm_context, indents);

        let obj_tree = parser.parse_object_tree();

        Ok(Self {
            dm_context,
            icon_cache,
            obj_tree,
        })
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
    minimap::generate(minimap_context, icon_cache)
        .map_err(|_| anyhow::anyhow!("An error occured during map rendering"))
}

pub fn render_map_regions(
    context: &RenderingContext,
    maps: &[dmm::Map],
    bounds: &[BoundingBox],
    render_passes: &[Box<dyn RenderPass>],
    output_dir: &Path,
    filename: &str,
    errors: &RenderingErrors,
) -> Result<()> {
    let objtree = &context.obj_tree;
    let icon_cache = &context.icon_cache;
    let _: Result<()> = maps
        .par_iter()
        .zip(bounds.par_iter())
        .enumerate()
        .map(|(idx, (map, bb))| {
            eprintln!("rendering map {}", idx);
            let image = render_map(objtree, icon_cache, map, bb, errors, render_passes)
                .with_context(|| format!("Rendering map {idx}"))?;

            let directory = format!("{}/{}", output_dir.display(), idx);

            std::fs::create_dir_all(&directory).context("Creating directories")?;
            image
                .to_file(format!("{}/{}", directory, filename).as_ref())
                .with_context(|| format!("Saving image {idx}"))?;
            Ok(())
        })
        .collect();
    Ok(())
}
