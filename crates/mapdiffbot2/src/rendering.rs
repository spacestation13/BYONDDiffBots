use std::{
    cmp::min,
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::RwLock,
};

extern crate dreammaker;

use diffbot_lib::tracing;

use diffbot_lib::github::github_types::FileDiff;
use dmm_tools::{dmm, minimap, render_passes::RenderPass, IconCache};
use dreammaker::objtree::ObjectTree;
use eyre::{Context, Result};
use image::{EncodableLayout, ImageBuffer, ImageEncoder};
use object_store::azure::MicrosoftAzure;
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
        tracing::info!(
            "Maps have different sizes: {:?} {:?}",
            left_dims,
            right_dims
        );
    }

    let max_y = min(left_dims.1, right_dims.1);
    let max_x = min(left_dims.0, right_dims.0);

    tracing::debug!("max_y: {max_y}, max_x: {max_x}");

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

    tracing::debug!(
        "Before expansion max: (right, top):({rightmost}, {topmost}), min: (left, bottom):({leftmost}, {bottommost})",
    );

    //this is a god awful way to expand bounds without it going out of bounds

    rightmost = rightmost.saturating_add(2).clamp(1, (max_x - 1).max(1));
    topmost = topmost.saturating_add(2).clamp(1, (max_y - 1).max(1));
    leftmost = leftmost.saturating_sub(2).clamp(1, (max_x - 1).max(1));
    bottommost = bottommost.saturating_sub(2).clamp(1, (max_y - 1).max(1));

    tracing::debug!(
        "After expansion max: (right, top):({rightmost}, {topmost}), min: (left, bottom):({leftmost}, {bottommost})",
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
                    .wrap_err_with(|| format!("Map name: {}", &file.filename)),
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
            .wrap_err("Creating preprocessor")?;
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
) -> Result<image::RgbaImage> {
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
        .map_err(|_| eyre::eyre!("An error occured during map rendering"))
}

#[derive(Clone, Copy)]
pub enum MapType {
    Head,
    Base,
}

pub type RenderedMaps = IndexMap<PathBuf, Vec<u8>, ahash::RandomState>;

pub fn render_map_regions<'a, 'b, M>(
    context: &RenderingContext,
    maps: M, //&[(&str, &MapWithRegions)],
    render_passes: &[Box<dyn RenderPass>],
    (output_dir, blob_client): (&Path, Azure),
    filename: &str,
    errors: &RenderingErrors,
    map_type: MapType,
) -> Result<RenderedMaps>
where
    M: ParallelIterator<Item = (&'a str, &'b MapWithRegions)>,
{
    let objtree = &context.obj_tree;
    let icon_cache = &context.icon_cache;
    let results = maps
        .map(|(map_name, map)| -> Result<RenderedMaps> {
            render_map_region(
                map_name,
                map,
                map_type,
                (objtree, icon_cache, errors, render_passes),
                (output_dir, blob_client.clone(), filename),
            )
        })
        .collect::<Vec<_>>();

    results.iter().for_each(|res| {
        if let Err(e) = res {
            tracing::error!("{e:?}") //errors please
        }
    });

    Ok(results
        .into_iter()
        .filter_map(|thing| thing.ok())
        .flatten()
        .collect::<RenderedMaps>())
}

fn render_map_region(
    map_name: &str,
    map: &MapWithRegions,
    map_type: MapType,
    (objtree, icon_cache, errors, render_passes): (
        &ObjectTree,
        &IconCache,
        &RenderingErrors,
        &[Box<dyn RenderPass>],
    ),
    (output_dir, blob_client, filename): (&Path, Azure, &str),
) -> Result<RenderedMaps> {
    let mut return_map: RenderedMaps = Default::default();
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
                .wrap_err_with(|| format!("Rendering map {map_name}"))?,
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
                    .wrap_err_with(|| format!("Rendering map {map_name}"))?,
                )
            }
            (_, _) => None,
        };

        tracing::debug!(
            "maprender: {map_name}, image: {}, azure: {}, path: {}",
            image.is_some(),
            blob_client.is_some(),
            output_dir.display(),
        );
        let directory = output_dir
            .join(Path::new(
                &map_name.to_string().replace('/', "_").replace(".dmm", ""),
            ))
            .join(Path::new(&format!("{z_level}-{filename}")));

        tracing::debug!("file at: {directory:?}");

        if let Some(image) = image {
            let compressed_image = compress_image(image).wrap_err("Failed to compress image")?;
            return_map.insert(directory.to_path_buf(), compressed_image);
        }
    }
    return_map.iter().for_each(|(directory, compressed_image)| {
        if let Some(ref blob_client) = blob_client {
            if let Err(e) =
                write_to_azure(directory, blob_client.clone(), compressed_image.as_slice())
                    .wrap_err("Sending image to azure")
            {
                tracing::error!("{e:?}")
            };
            tracing::debug!("Sent to azure: {map_name} {directory:?}");
        } else {
            if let Err(e) = write_to_file(directory, compressed_image.as_slice())
                .wrap_err("Writing image to file")
            {
                tracing::error!("{e:?}")
            };
            tracing::debug!("Wrote to file: {map_name} {directory:?}");
        }
    });

    Ok(return_map)
}

pub fn render_diffs(before: RenderedMaps, after: RenderedMaps, blob_client: Azure) {
    let res = before
        .par_iter()
        .filter_map(|before| {
            let after_path = replace_name_pathbuf(before.0, "-before.png", "-after.png");
            let after = after.get_key_value(&after_path)?;
            Some((before, after))
        })
        .map(
            |((before_path, before_image), (_, after_image))| -> Result<(PathBuf, Vec<u8>)> {
                let (before_image, after_image) = (
                    decode_image(before_image).wrap_err("Failed to decode before image")?,
                    decode_image(after_image).wrap_err("Failed to decode after image")?,
                );

                let image =
                    ImageBuffer::from_fn(after_image.width(), after_image.height(), |x, y| {
                        use image::Pixel;
                        let before_pixel = before_image.get_pixel(x, y);
                        let after_pixel = after_image.get_pixel(x, y);
                        if before_pixel == after_pixel {
                            after_pixel.map_without_alpha(|c| c.saturating_add((255 - c) / 3))
                        } else {
                            image::Rgba([255, 0, 0, 255])
                        }
                    });

                let final_path = replace_name_pathbuf(before_path, "-before.png", "-diff.png");

                let image = compress_image(image).wrap_err("Failed to compress image")?;

                Ok((final_path, image))
            },
        )
        .collect::<Vec<_>>();

    res.iter()
        .filter_map(|item| item.as_ref().ok())
        .for_each(|(final_path, image)| {
            if let Some(client) = blob_client.clone() {
                if let Err(e) = write_to_azure(final_path, client, image.as_slice())
                    .wrap_err("Sending image to azure")
                {
                    tracing::error!("{e:?}")
                };
                tracing::debug!("Sent to azure: {final_path:?}");
            } else {
                if let Err(e) =
                    write_to_file(final_path, image.as_slice()).wrap_err("Writing image to file")
                {
                    tracing::error!("{e:?}")
                };
                tracing::debug!("Written to file: {final_path:?}");
            }
        });

    res.into_iter().for_each(|res| {
        if let Err(e) = res {
            tracing::error!("{e:?}");
        }
    });
}

fn write_to_azure<P: AsRef<Path>>(
    path: P,
    client: std::sync::Arc<MicrosoftAzure>,
    compressed_image: &[u8],
) -> Result<()> {
    use object_store::ObjectStore;

    let path = object_store::path::Path::from_iter(
        path.as_ref().iter().map(|ostr| ostr.to_str().unwrap()),
    );

    let handle = actix_web::rt::Runtime::new().wrap_err("Failed to get an actixweb runtime")?;

    let blob_client = client.clone();

    handle
        .block_on(blob_client.put(&path, compressed_image.to_owned().into()))
        .wrap_err("Failed to put a block blob into azure")?;
    Ok(())
}

fn compress_image(image: image::RgbaImage) -> Result<Vec<u8>> {
    let mut vec = vec![];
    encode_image(&mut vec, &image).wrap_err("Failed to encode image")?;
    Ok(vec)
}

fn write_to_file<P: AsRef<Path>>(path: P, compressed_image: &[u8]) -> Result<()> {
    std::fs::create_dir_all(
        path.as_ref()
            .parent()
            .ok_or_else(|| eyre::eyre!("Path has no parent!"))?,
    )
    .wrap_err("Failed to create dir")?;

    let mut file = std::io::BufWriter::new(
        std::fs::File::create(path).wrap_err("Failed to create image file")?,
    );

    file.write_all(compressed_image)
        .wrap_err("Failed to write image to file")?;

    Ok(())
}
fn encode_image<W: std::io::Write>(write_to: &mut W, image: &image::RgbaImage) -> Result<()> {
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        write_to,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::Adaptive,
    );
    encoder
        .write_image(
            image.as_bytes(),
            image.width(),
            image.height(),
            image::ColorType::Rgba8,
        )
        .wrap_err("Failed to encode image")?;
    Ok(())
}

fn decode_image(bytes: &[u8]) -> Result<image::RgbaImage> {
    let image = image::io::Reader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .wrap_err("Failed to guess image format")?
        .decode()
        .wrap_err("Failed to decode image")?;
    let image = image.to_rgba8();
    Ok(image)
}

fn replace_name_pathbuf<P: AsRef<Path>>(buf: P, from: &str, to: &str) -> PathBuf {
    let mut return_path = buf.as_ref().to_path_buf();
    let filename = return_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .replace(from, to);
    return_path.set_file_name(filename);
    return_path
}
