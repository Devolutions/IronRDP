// pub mod framed;

#[cfg(all(feature = "tokio", feature = "futures"))]
compile_error!("Only \"tokio\" or \"futures\" should be enabled at a time.");
