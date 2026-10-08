use crate::{config::Config, host};
use anyhow::{Context, Result};
use starlark::{
    environment::{GlobalsBuilder, Module},
    eval::Evaluator,
    syntax::{AstModule, Dialect},
};

pub fn run(config: &Config, name: &str, args: &[String]) -> Result<()> {
    let applet = config.applets.get(name).context("Unknown applet")?;
    let filename = format!("{}[applets.{name}.script]", config.source.display());
    let ast = AstModule::parse(&filename, applet.script.clone(), &Dialect::Standard)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let globals = GlobalsBuilder::standard().with(host::functions).build();
    Module::with_temp_heap(|module| {
        let mut evaluator = Evaluator::new(&module);
        evaluator.extra = Some(config);
        evaluator
            .eval_module(ast, &globals)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let main = module
            .get("main")
            .context("Script must define main(args)")?;
        evaluator
            .eval_function(main, &[module.heap().alloc(args.to_vec())], &[])
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok::<(), anyhow::Error>(())
    })
    .with_context(|| format!("Script {filename}"))
}
