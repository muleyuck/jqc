use anyhow::{anyhow, Result};
use jsonc_parser::cst::CstRootNode;
use jsonc_parser::ParseOptions;

pub fn parse(text: &str) -> Result<CstRootNode> {
    CstRootNode::parse(text, &ParseOptions::default())
        .map_err(|e| anyhow!("Failed to parse JSONC: {}", e))
}
