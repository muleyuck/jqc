use anyhow::{anyhow, Result};
use jaq_core::load::{Arena, File, Loader};
use jaq_core::{data, Compiler, Ctx, Vars};
use jaq_json::{read, Val};
use serde_json::Value;

/// Apply a jq filter string to a JSON value and return a list of output values.
/// The input is accepted as `serde_json::Value` and converted to `jaq_json::Val` internally.
pub fn run_filter(filter_str: &str, input: Value) -> Result<Vec<Val>> {
    // serde_json::Value → JSON string → jaq_json::Val
    let json_str = serde_json::to_string(&input)?;
    let input_val = read::parse_single(json_str.as_bytes())
        .map_err(|e| anyhow!("Failed to convert value: {}", e))?;

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
