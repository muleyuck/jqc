use anyhow::{anyhow, Result};
use jaq_core::load::{Arena, File, Loader};
use jaq_core::{data, Compiler, Ctx, Vars};
use jaq_json::Val;
use jsonc_parser::ParseOptions;

/// Parse JSONC text and apply a jq filter, returning a list of output values.
/// Parses directly from JSONC to `Val` in a single step via serde Deserialize.
pub fn run_filter(filter_str: &str, text: &str) -> Result<Vec<Val>> {
    // JSONC text → jaq_json::Val directly (no intermediate serde_json::Value or string round-trip)
    let input_val = jsonc_parser::parse_to_serde_value::<Val>(text, &ParseOptions::default())
        .map_err(|e| anyhow!("Failed to parse JSONC: {}", e))?;

    // Collect defs (named filter definitions) and funs (native functions)
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let funs = jaq_core::funs::<data::JustLut<Val>>()
        .chain(jaq_std::funs::<data::JustLut<Val>>())
        .chain(jaq_json::funs::<data::JustLut<Val>>());

    // Parse and load the filter
    let program = File {
        code: filter_str,
        path: (),
    };
    let loader = Loader::new(defs);
    let arena = Arena::default();
    let modules = loader
        .load(&arena, program)
        .map_err(|e| anyhow!("Filter parse error: {:?}", e))?;

    // Compile the filter
    let filter = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|e| anyhow!("Filter compile error: {:?}", e))?;

    // Execute the filter
    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    let results = filter
        .id
        .run((ctx, input_val))
        .map(|r| r.map_err(|e| anyhow!("Filter runtime error: {:?}", e)))
        .collect::<Result<Vec<_>>>()?;

    Ok(results)
}
