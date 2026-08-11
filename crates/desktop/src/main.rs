mod ui;

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use gittles_core::{GitHub, Store, auth};
use gpui::{
    AnyView, App, AppContext, Application, Bounds, Focusable, WindowBounds, WindowOptions, px, size,
};

use gpui_component::{
    Root,
    theme::{Theme, ThemeMode},
};
use ui::Browser;

#[derive(Parser)]
#[command(name = "gittles", about = "Browse your GitHub stars", version)]
struct Args {
    /// Pull your stars from GitHub, then exit.
    #[arg(long)]
    sync: bool,

    /// Stop at this many stars. 0 means all of them.
    #[arg(long, default_value_t = 0, value_name = "N")]
    limit: usize,

    /// Forget the stored token, then exit.
    #[arg(long)]
    logout: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let store = Store::discover()?;

    if args.logout {
        store.clear_auth()?;
        println!("signed out");
        return Ok(());
    }

    // reqwest needs a tokio reactor, and gpui's executor is not one. Everything
    // that touches the network runs here; results cross back into the UI over a
    // channel.
    let runtime = Arc::new(tokio::runtime::Runtime::new()?);

    if args.sync {
        return runtime.block_on(sync(&store, args.limit));
    }

    browse(store, runtime);
    Ok(())
}

/// Device-flow login, printed to the terminal. The desktop app can browse a
/// cached list without a token, so this is only reached on an explicit sync.
async fn ensure_token(store: &Store) -> Result<String> {
    let token = store.load_config().token;
    if !token.is_empty() {
        return Ok(token);
    }

    let device = auth::request_device_code().await?;
    println!();
    println!("  1. open {}", device.verification_uri);
    println!("  2. enter the code {}", device.user_code);
    println!();
    println!("waiting for authorization…");

    let mut interval = device.interval();
    let deadline = std::time::Instant::now() + device.expires_in();

    let token = loop {
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("device code expired — run `gittles --sync` again");
        }

        tokio::time::sleep(interval).await;

        let outcome = auth::poll_once(&device.device_code).await?;
        interval = auth::next_interval(interval, &outcome);

        if let auth::Poll::Authorized(token) = outcome {
            break token;
        }
    };

    let github = GitHub::new(&token)?;
    let username = github.username().await?;

    let config = store.load_config();
    store.save_config(&gittles_core::Config {
        token: token.clone(),
        username: username.clone(),
        ..config
    })?;

    println!("authorized as {username}");
    Ok(token)
}

async fn sync(store: &Store, limit: usize) -> Result<()> {
    let token = ensure_token(store).await?;
    let previous: std::collections::HashSet<u64> =
        store.load_stars().into_iter().map(|star| star.id).collect();

    let github = GitHub::new(token)?;
    let stars = github
        .stars(limit, |fetched, page| {
            println!("fetched {fetched} stars (page {page})…");
        })
        .await?;

    let current: std::collections::HashSet<u64> = stars.iter().map(|star| star.id).collect();
    let added = stars
        .iter()
        .filter(|star| !previous.contains(&star.id))
        .count();
    let removed = previous.difference(&current).count();

    store.save_stars(&stars)?;
    store.mark_synced(jiff::Timestamp::now().to_string())?;

    println!(
        "synced {} stars  +{added} new  -{removed} gone",
        stars.len()
    );
    Ok(())
}

fn browse(store: Store, runtime: Arc<tokio::runtime::Runtime>) {
    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        // gpui-component defaults to a light theme; its widget chrome would
        // otherwise sit as a white box in the middle of gittles' dark palette.
        Theme::change(ThemeMode::Dark, None, cx);

        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let browser = cx.new(|cx| Browser::new(store.clone(), runtime.clone(), window, cx));
                let focus = browser.focus_handle(cx);
                window.focus(&focus);
                // gpui-component's widgets reach for `Root` as the window's first
                // layer — the text input panics outright without it.
                cx.new(|cx| Root::new(AnyView::from(browser), window, cx))
            },
        )
        .expect("failed to open window");

        cx.activate(true);
    });
}
