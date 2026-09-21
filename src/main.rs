#[cfg(not(target_arch = "wasm32"))]
mod app;

/// All URLs are relative, so the export also works at a Pages repository path.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os().nth(1).unwrap_or_else(|| "dist".into());
    let output = std::path::Path::new(&output);
    let html = app::render().map_err(|error| std::io::Error::other(error.to_string()))?;
    std::fs::create_dir_all(output)?;
    std::fs::write(output.join("index.html"), html)?;
    std::fs::write(output.join(".nojekyll"), "")?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
