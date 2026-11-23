//! A development check of the update pipeline against a local fake release server (never GitHub): the check, the
//! download and every verification, stopping before the install. Debug builds only (`LAUNCHPAD_DESKTOP_UPDATE_API`).
//!
//! `LAUNCHPAD_DESKTOP_UPDATE_API=http://127.0.0.1:<port> XDG_CACHE_HOME=<scratch> cargo run --example update_probe`

use launchpad_desktop::update;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let http = update::client();
    println!("this build: {} ({}), API {}", update::version(), update::channel(), update::api_base());
    let found = update::check(&http, "release").await?;
    let Some(c) = found else {
        println!("no newer package");
        return Ok(());
    };
    println!("newer package: {} ({})", c.version, c.deb.name);
    match update::download(&http, &c, || println!("downloaded; verifying…")).await {
        Ok(path) => {
            println!("verified: {} — stopping before the install", path.display());
            let _ = std::fs::remove_dir_all(path.parent().unwrap());
        }
        Err(e) => println!("refused: {e}"),
    }
    Ok(())
}
