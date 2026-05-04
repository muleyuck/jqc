use anyhow::{anyhow, Result};
use jaq_core::load::{Arena, File, Loader};
use jaq_core::{data, Compiler, Ctx, Vars};
use jaq_json::Val;
use jsonc_parser::ParseOptions;

/// Parse `text` as JSONC and apply `filter_str` as a jq filter.
/// Returns all output values produced by the filter.
pub fn run(filter_str: &str, text: &str) -> Result<Vec<Val>> {
    let input_val = jsonc_parser::parse_to_serde_value::<Val>(text, &ParseOptions::default())
        .map_err(|e| anyhow!("Failed to parse JSONC: {e}"))?;

    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let funs = jaq_core::funs::<data::JustLut<Val>>()
        .chain(jaq_std::funs::<data::JustLut<Val>>())
        .chain(jaq_json::funs::<data::JustLut<Val>>());

    let program = File { code: filter_str, path: () };
    let loader = Loader::new(defs);
    let arena = Arena::default();
    let modules = loader
        .load(&arena, program)
        .map_err(|e| anyhow!("Filter parse error: {e:?}"))?;
    let filter = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|e| anyhow!("Filter compile error: {e:?}"))?;

    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    filter
        .id
        .run((ctx, input_val))
        .map(|r| r.map_err(|e| anyhow!("Filter runtime error: {e:?}")))
        .collect()
}
