use std::{cmp::min, collections::HashSet, path::Path, sync::RwLock};

extern crate dreammaker;

use diffbot_lib::log;

use diffbot_lib::github::github_types::FileDiff;
use dmm_tools::{dmi::Image, dmm, minimap, render_passes::RenderPass, IconCache};
use eyre::{Context, Result};
use image::{io::Reader, ImageBuffer};
use rayon::prelude::*;

use ahash::RandomState;
use indexmap::IndexMap;

use super::Azure;

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
        log::info!(
            "Maps have different sizes: {:?} {:?}",
            left_dims,
            right_dims
        );
    }

    let max_y = min(left_dims.1, right_dims.1);
    let max_x = min(left_dims.0, right_dims.0);

    log::debug!("max_y: {}, max_x: {}", max_y, max_x);

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

    log::debug!(
        "Before expansion max: (right, top):({}, {}), min: (left, bottom):({}, {})",
        rightmost,
        topmost,
        leftmost,
        bottommost
    );

    //this is a god awful way to expand bounds without it going out of bounds

    rightmost = rightmost.saturating_add(2).clamp(1, (max_x - 1).max(1));
    topmost = topmost.saturating_add(2).clamp(1, (max_y - 1).max(1));
    leftmost = leftmost.saturating_sub(2).clamp(1, (max_x - 1).max(1));
    bottommost = bottommost.saturating_sub(2).clamp(1, (max_y - 1).max(1));

    log::debug!(
        "After expansion max: (right, top):({}, {}), min: (left, bottom):({}, {})",
        rightmost,
        topmost,
        leftmost,
        bottommost
    );

    Some(BoundingBox::new(leftmost, bottommost, rightmost, topmost))
}

pub fn load_maps(
    files: &[&FileDiff],
    path: &std::path::Path,
) -> IndexMap<String, Result<dmm::Map>, RandomState> {
    files
        .iter()
        .map(|file| {
            let actual_path = path.join(Path::new(&file.filename));
            (
                file.filename.clone(),
                dmm::Map::from_file(&actual_path)
                    .map_err(|e| eyre::anyhow!(e))
                    .context(format!("Map name: {}", &file.filename)),
            )
        })
        .collect()
}

pub fn load_maps_with_whole_map_regions(
    files: &[&FileDiff],
    path: &std::path::Path,
) -> Result<Vec<(String, MapWithRegions)>> {
    files
        .iter()
        .map(|file| {
            let actual_path = path.join(Path::new(&file.filename));
            let map = dmm::Map::from_file(&actual_path)?;
            let bbox = BoundingBox::for_full_map(&map);
            let zs = map.dim_z();
            Ok((
                file.filename.clone(),
                MapWithRegions {
                    map,
                    bounding_boxes: std::iter::repeat(BoundType::Both(bbox)).take(zs).collect(),
                },
            ))
        })
        .collect()
}

#[derive(Clone)]
pub enum BoundType {
    OnlyHead,
    OnlyBase,
    Both(BoundingBox),
    None,
}

pub struct MapWithRegions {
    pub map: dmm::Map,
    /// For each z-level, if there's a Some, render the given region
    pub bounding_boxes: Vec<BoundType>,
}
// pub fn iter_levels<'a>(&'a self) -> impl Iterator<Item=(i32, ZLevel<'a>)> + 'a {
impl MapWithRegions {
    pub fn iter_levels(&self) -> impl Iterator<Item = (usize, &BoundType)> {
        self.bounding_boxes
            .iter()
            .enumerate()
            .filter_map(|(z, bbox)| {
                if matches!(bbox, BoundType::None) {
                    None
                } else {
                    Some((z, bbox))
                }
            })
    }
}

pub type MapsWithRegions = IndexMap<String, (RegionsBefore, RegionsAfter), RandomState>;
pub type RegionsAfter = Option<MapWithRegions>;
pub type RegionsBefore = Result<MapWithRegions>;

pub fn get_map_diff_bounding_boxes(
    modified_maps: IndexMap<String, (Result<dmm::Map>, Result<dmm::Map>), RandomState>,
) -> Result<MapsWithRegions> {
    use itertools::{EitherOrBoth, Itertools};

    let mut returned_maps =
        IndexMap::with_capacity_and_hasher(modified_maps.len(), ahash::RandomState::default());

    for (map_name, (base, head)) in modified_maps.into_iter() {
        match (base, head) {
            (Ok(base), Ok(head)) => {
                let diffs = (0..base.dim_z())
                    .zip_longest(0..head.dim_z())
                    .map(|either| match either {
                        EitherOrBoth::Both(z, _) => match get_diff_bounding_box(&base, &head, z) {
                            Some(boxed) => BoundType::Both(boxed),
                            None => BoundType::None,
                        },
                        EitherOrBoth::Left(_base_only) => BoundType::OnlyBase,
                        EitherOrBoth::Right(_head_only) => BoundType::OnlyHead,
                    })
                    .collect::<Vec<_>>();
                let before = MapWithRegions {
                    map: base,
                    bounding_boxes: diffs.clone(),
                };
                let after = MapWithRegions {
                    map: head,
                    bounding_boxes: diffs,
                };
                returned_maps.insert(map_name, (Ok(before), Some(after)));
                Ok(())
            }
            (Err(e), _) => {
                returned_maps.insert(map_name, (Err(e), None));
                Ok(())
            }
            (_, Err(e)) => Err(e), //Fails on head parse fail
        }?; //Stop the entire thing if head parse fails
    }

    Ok(returned_maps)
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
    let arena = Default::default();
    let minimap_context = minimap::Context {
        objtree,
        map,
        level: map.z_level(z_level),
        min: (bounds.left, bounds.bottom),
        max: (bounds.right, bounds.top),
        render_passes,
        errors,
        arena: &arena,
    };
    minimap::generate(minimap_context, icon_cache)
        .map_err(|_| eyre::anyhow!("An error occured during map rendering"))
}

#[derive(Clone, Copy)]
pub enum MapType {
    Head,
    Base,
}

pub fn render_map_regions(
    context: &RenderingContext,
    maps: &[(&str, &MapWithRegions)],
    render_passes: &[Box<dyn RenderPass>],
    (output_dir, blob_client): (&Path, Azure),
    filename: &str,
    errors: &RenderingErrors,
    map_type: MapType,
) -> Result<()> {
    let objtree = &context.obj_tree;
    let icon_cache = &context.icon_cache;
    let results = maps
        .par_iter()
        .map(|(map_name, map)| -> Result<()> {
            for z_level in 0..map.map.dim_z() {
                let image = match (
                    map_type,
                    map.bounding_boxes
                        .get(z_level)
                        .expect("No bounding box generated for z-level"),
                ) {
                    (_, BoundType::Both(bounds)) => Some(
                        render_map(
                            objtree,
                            icon_cache,
                            &map.map,
                            z_level,
                            bounds,
                            errors,
                            render_passes,
                        )
                        .with_context(|| format!("Rendering map {map_name}"))?,
                    ),
                    (MapType::Head, BoundType::OnlyHead) => {
                        let bounds = BoundingBox::for_full_map(&map.map);
                        Some(
                            render_map(
                                objtree,
                                icon_cache,
                                &map.map,
                                z_level,
                                &bounds,
                                errors,
                                render_passes,
                            )
                            .with_context(|| format!("Rendering map {map_name}"))?,
                        )
                    }
                    (_, _) => None,
                };

                log::debug!(
                    "maprender: {map_name} : {}, azure: {}, path: {}",
                    image.is_some(),
                    blob_client.is_some(),
                    output_dir.display(),
                );

                match (image, blob_client.as_ref()) {
                    (Some(image), Some(blob_client)) => {
                        use object_store::ObjectStore;

                        let directory = output_dir
                            .join(Path::new(
                                &map_name.to_string().replace('/', "_").replace(".dmm", ""),
                            ))
                            .join(Path::new(&format!("{z_level}-{filename}")));

                        let bytes = image.to_bytes()?;

                        let path = object_store::path::Path::from_iter(
                            directory.iter().map(|ostr| ostr.to_str().unwrap()),
                        );

                        let handle = actix_web::rt::Runtime::new()?;

                        let blob_client = blob_client.clone();

                        handle.block_on(async move {
                            use tokio::io::AsyncWriteExt;
                            let (_, mut multipart) =
                                blob_client.put_multipart(&path).await.unwrap();
                            //for thing in bytes.chunks(1_000_000).map(|item| item.to_vec()) {
                            //blob_client.put(&path, thing.into()).await.unwrap();
                            //multipart.write(thing.as_slice()).await;
                            multipart.write_all(bytes.as_slice()).await.unwrap();
                            multipart.flush().await.unwrap();
                            multipart.shutdown().await.unwrap();

                            //}
                        });
                        log::debug!("Sent to azure: {map_name}");
                    }
                    (Some(image), None) => {
                        let directory = output_dir.join(Path::new(
                            &map_name.to_string().replace('/', "_").replace(".dmm", ""),
                        ));

                        std::fs::create_dir_all(&directory).context("Creating directories")?;
                        image.to_file(
                            &directory.join(Path::new(&format!("{z_level}-{filename}"))),
                        )?;
                    }
                    (_, _) => (),
                }
            }
            Ok(())
        })
        .collect::<Vec<_>>();
    results.iter().for_each(|res| {
        if let Err(e) = res {
            log::error!("{:?}", e) //errors please
        }
    });
    for thing in results {
        thing?; //henlo?????
    }
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
            let before = before
                .as_rgba8()
                .ok_or_else(|| eyre::eyre!("Image is not rgba8!"))?;
            let after = Reader::open(replaced_entry)?.decode()?;
            let after = after
                .as_rgba8()
                .ok_or_else(|| eyre::eyre!("Image is not rgba8!"))?;

            ImageBuffer::from_fn(after.width(), after.height(), |x, y| {
                use image::Pixel;
                let before_pixel = before.get_pixel(x, y);
                let after_pixel = after.get_pixel(x, y);
                if before_pixel == after_pixel {
                    after_pixel.map_without_alpha(|c| c.saturating_add((255 - c) / 3))
                } else {
                    image::Rgba([255, 0, 0, 255])
                }
            })
            .save(fuck.replace("-before.png", "-diff.png"))?;

            Ok(())
        })
        .filter_map(|r: Result<()>| r.err())
        .for_each(|e| {
            log::error!("Diff rendering error: {}", e);
        });
}
