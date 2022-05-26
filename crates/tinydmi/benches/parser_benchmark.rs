use criterion::{criterion_group, criterion_main, Criterion};
use tinydmi::parser::Metadata;

static SMOL_DATA: &str = r#"# BEGIN DMI
version = 4.0
	width = 32
	height = 32
state = "cocktail"
	dirs = 1
	frames = 1
# END DMI"#;

static DRINKS: &str = include_str!("files/drinks.txt");
static AI: &str = include_str!("files/paradise-ai.txt");
static HEAD: &str = include_str!("files/paradise-head.txt");
static LEFTHAND: &str = include_str!("files/paradise-lefthand.txt");
static ROBOTS: &str = include_str!("files/paradise-robots.txt");
static SUIT: &str = include_str!("files/paradise-suit.txt");

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("Small example", |b| {
        b.iter_with_large_drop(|| Metadata::load(&SMOL_DATA).unwrap())
    });
    c.bench_function("CHOMP - drinks.dmi", |b| {
        b.iter_with_large_drop(|| Metadata::load(&DRINKS).unwrap())
    });
    c.bench_function("Paradise - ai.dmi", |b| {
        b.iter_with_large_drop(|| Metadata::load(&AI).unwrap())
    });
    c.bench_function("Paradise - head.dmi", |b| {
        b.iter_with_large_drop(|| Metadata::load(&HEAD).unwrap())
    });
    c.bench_function("Paradise - lefthand.dmi", |b| {
        b.iter_with_large_drop(|| Metadata::load(&LEFTHAND).unwrap())
    });
    c.bench_function("Paradise - robots.dmi", |b| {
        b.iter_with_large_drop(|| Metadata::load(&ROBOTS).unwrap())
    });
    c.bench_function("Paradise - suit.dmi", |b| {
        b.iter_with_large_drop(|| Metadata::load(&SUIT).unwrap())
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
