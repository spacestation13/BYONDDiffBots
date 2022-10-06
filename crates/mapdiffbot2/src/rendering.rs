use std::{cmp::min, collections::HashSet, path::Path, sync::RwLock};

extern crate dreammaker;

use ahash::RandomState;
use anyhow::{Context, Result};
use diffbot_lib::github::github_types::FileDiff;
use dmm_tools::{dmi::Image, dmm, minimap, render_passes::RenderPass, IconCache};
use image::{io::Reader, GenericImageView, ImageBuffer, Pixel};
use rayon::prelude::*;

#[derive(Debug, Clone)]
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

impl ToString for BoundingBox {
    fn to_string(&self) -> String {
        format!(
            "({}, {}) -> ({}, {})",
            self.left, self.bottom, self.right, self.top
        )
    }
}

pub type RenderingErrors = RwLock<HashSet<String, RandomState>>;

// Returns None if there are no differences
pub fn get_diff_bounding_box(
    base_map: &dmm::Map,
    head_map: &dmm::Map,
    z_level: usize,
) -> Option<BoundingBox> {
    let left_dims = base_map.dim_xyz();
    let right_dims = head_map.dim_xyz();
    if left_dims != right_dims {
        println!(
            "Maps have different sizes: {:?} {:?}",
            left_dims, right_dims
        );
    }

    let max_y = min(left_dims.1, right_dims.1);
    let max_x = min(left_dims.0, right_dims.0);

    dbg!("max_y: {}, max_x: {}", max_y, max_x);

    let mut rightmost = 0usize;
    let mut leftmost = max_x;
    let mut topmost = 0usize;
    let mut bottommost = max_y;

    for y in 0..max_y {
        for x in 0..max_x {
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
        return None;
    }

    dbg!(
        "Before expansion max: (right, top):({}, {}), min: (left, bottom):({}, {})",
        rightmost,
        topmost,
        leftmost,
        bottommost
    );

    //this is a god awful way to expand bounds without it going out of bounds

    rightmost = rightmost.saturating_add(2).clamp(1, max_x - 1);
    topmost = topmost.saturating_add(2).clamp(1, max_y - 1);
    leftmost = leftmost.saturating_sub(2).clamp(1, max_x - 1);
    bottommost = bottommost.saturating_sub(2).clamp(1, max_y - 1);

    dbg!(
        "After expansion max: (right, top):({}, {}), min: (left, bottom):({}, {})",
        rightmost,
        topmost,
        leftmost,
        bottommost
    );

    Some(BoundingBox::new(leftmost, bottommost, rightmost, topmost))
}

pub fn load_maps(files: &[&FileDiff]) -> Result<Vec<dmm::Map>> {
    files
        .iter()
        .map(|file| dmm::Map::from_file(Path::new(&file.filename)).map_err(|e| anyhow::anyhow!(e)))
        .collect()
}

pub fn load_maps_with_whole_map_regions(files: &[&FileDiff]) -> Result<Vec<MapWithRegions>> {
    files
        .iter()
        .map(|file| {
            let map = dmm::Map::from_file(Path::new(&file.filename))?;
            let bbox = BoundingBox::for_full_map(&map);
            let zs = map.dim_z();
            Ok(MapWithRegions {
                map,
                bounding_boxes: std::iter::repeat(Some(bbox)).take(zs).collect(),
            })
        })
        .collect()
}

pub struct MapWithRegions {
    pub map: dmm::Map,
    /// For each z-level, if there's a Some, render the given region
    pub bounding_boxes: Vec<Option<BoundingBox>>,
}

// pub fn iter_levels<'a>(&'a self) -> impl Iterator<Item=(i32, ZLevel<'a>)> + 'a {
impl MapWithRegions {
    pub fn iter_levels(&self) -> impl Iterator<Item = (usize, &BoundingBox)> {
        self.bounding_boxes
            .iter()
            .enumerate()
            .filter_map(|(z, bbox)| bbox.as_ref().map(|bbox| (z, bbox)))
    }
}

pub struct MapsWithRegions {
    pub befores: Vec<MapWithRegions>,
    pub afters: Vec<MapWithRegions>,
}

pub fn get_map_diff_bounding_boxes(
    base_maps: Vec<dmm::Map>,
    head_maps: Vec<dmm::Map>,
) -> MapsWithRegions {
    let (befores, afters) = base_maps
        .into_par_iter()
        .zip(head_maps.into_par_iter())
        .map(|(base, head)| {
            let diffs = (0..base.dim_z())
                .map(|z| get_diff_bounding_box(&base, &head, z))
                .collect::<Vec<_>>();
            let before = MapWithRegions {
                map: base,
                bounding_boxes: diffs.clone(),
            };
            let after = MapWithRegions {
                map: head,
                bounding_boxes: diffs,
            };
            (before, after)
        })
        .collect();

    MapsWithRegions { befores, afters }
}

pub struct RenderingContext {
    map_renderer_config: dreammaker::config::MapRenderer,
    obj_tree: dreammaker::objtree::ObjectTree,
    icon_cache: IconCache,
}

impl RenderingContext {
    pub fn new(path: &Path) -> Result<Self> {
        let dm_context = dreammaker::Context::default();
        let mut icon_cache = IconCache::default();

        let environment = match dreammaker::detect_environment(path, dreammaker::DEFAULT_ENV) {
            Ok(Some(found)) => found,
            _ => dreammaker::DEFAULT_ENV.into(),
        };

        if let Some(parent) = environment.parent() {
            icon_cache.set_icons_root(parent);
        }

        dm_context.autodetect_config(&environment);
        let pp = dreammaker::preprocessor::Preprocessor::new(&dm_context, environment)
            .context("Creating preprocessor")?;
        let indents = dreammaker::indents::IndentProcessor::new(&dm_context, pp);
        let parser = dreammaker::parser::Parser::new(&dm_context, indents);

        let obj_tree = parser.parse_object_tree();
        let map_renderer_config = dm_context.config().map_renderer.clone();

        Ok(Self {
            map_renderer_config,
            icon_cache,
            obj_tree,
        })
    }

    pub fn map_config(&self) -> &dreammaker::config::MapRenderer {
        &self.map_renderer_config
    }
}

pub fn render_map(
    objtree: &dreammaker::objtree::ObjectTree,
    icon_cache: &IconCache,
    map: &dmm::Map,
    z_level: usize,
    bounds: &BoundingBox,
    errors: &RwLock<HashSet<String, RandomState>>,
    render_passes: &[Box<dyn RenderPass>],
) -> Result<Image> {
    let bump = Default::default();
    let minimap_context = minimap::Context {
        objtree,
        map,
        level: map.z_level(z_level),
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
    maps: &[MapWithRegions],
    render_passes: &[Box<dyn RenderPass>],
    output_dir: &Path,
    filename: &str,
    errors: &RenderingErrors,
) -> Result<()> {
    let objtree = &context.obj_tree;
    let icon_cache = &context.icon_cache;
    let _: Result<()> = maps
        .par_iter()
        .enumerate()
        .map(|(idx, map)| {
            for z_level in 0..map.map.dim_z() {
                if let Some(bounds) = map
                    .bounding_boxes
                    .get(z_level)
                    .expect("No bounding box generated for z-level")
                {
                    let image = render_map(
                        objtree,
                        icon_cache,
                        &map.map,
                        z_level,
                        bounds,
                        errors,
                        render_passes,
                    )
                    .with_context(|| format!("Rendering map {idx}"))?;

                    let directory = format!("{}/{}", output_dir.display(), idx);

                    std::fs::create_dir_all(&directory).context("Creating directories")?;
                    image
                        .to_file(format!("{}/{}-{}", directory, z_level, filename).as_ref())
                        .with_context(|| format!("Saving image {idx}"))?;
                }
            }
            Ok(())
        })
        .collect();
    Ok(())
}

pub fn render_diffs_for_directory<P: AsRef<Path>>(directory: P) {
    let directory = directory.as_ref();

    glob::glob(directory.join("*-before.png").to_str().unwrap())
        .expect("Failed to read glob pattern")
        .filter_map(|f| f.ok())
        .par_bridge()
        .map(|entry| {
            let fuck = entry.to_string_lossy();
            let replaced_entry = fuck.replace("-before.png", "-after.png");
            let before = Reader::open(&entry)?.decode()?;
            let after = Reader::open(&replaced_entry)?.decode()?;

            ImageBuffer::from_fn(after.width(), after.height(), |x, y| {
                let before_pixel = before.get_pixel(x, y);
                let after_pixel = after.get_pixel(x, y);
                if before_pixel == after_pixel {
                    after_pixel
                        .map_without_alpha(|c| c.saturating_add((255 - c).saturating_mul(2 / 3)))
                } else {
                    image::Rgba([255, 0, 0, 255])
                }
            })
            .save(fuck.replace("-before.png", "-diff.png"))?;

            Ok(())
        })
        .filter_map(|r: Result<()>| r.err())
        .for_each(|e| {
            eprintln!("Diff rendering error: {}", e);
        });
}
