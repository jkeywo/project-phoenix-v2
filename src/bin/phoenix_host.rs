//! `phoenix-host` — serve the Phoenix client, manifest and catalogue natively.
//!
//! All the work lives in `project_phoenix::delivery` so it can be unit-tested;
//! this is just argv in, an operator log out, an exit code out.
//!
//! Exit codes: 0 clean, 1 startup failure (unreadable content, an unusable
//! client bundle, a version-pin mismatch against the bundle, a port already in
//! use), 2 bad arguments.
//!
//! Printing: `eprintln!` rather than the `plog!` family, on the same footing as
//! `phoenix-headless` — this is a CLI's own operator output, not simulation
//! logging, and there is no Bevy `App` here to hold a `LogFilterConfig`.

// `required-features` in Cargo.toml gates on features, not targets, so
// `--features host --target wasm32-unknown-unknown` would otherwise try to
// build this against a `crate::delivery::serve` that does not exist there.
#[cfg(target_arch = "wasm32")]
fn main() {
    eprintln!("phoenix-host is a native binary; it has no wasm32 build.");
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use project_phoenix::delivery::args::{parse_args, ClientSource, ParseOutcome, HELP};
    use project_phoenix::delivery::serve::{preload_templates, HostEvent, HostServer};

    let args = match parse_args(std::env::args().skip(1)) {
        Ok(ParseOutcome::Help) => {
            print!("{HELP}");
            return;
        }
        Ok(ParseOutcome::Run(args)) => *args,
        Err(e) => {
            eprintln!("phoenix-host: {e}");
            eprintln!("try --help");
            std::process::exit(2);
        }
    };

    // Enrichment only: the catalogue is correct without it, and a content tree
    // with no templates is a warning rather than a failure.
    match preload_templates(&args.content_dir) {
        Ok(0) => eprintln!(
            "phoenix-host: no entity templates under {}/assets/entities — the catalogue \
             will publish hull paths without class/rating metadata",
            args.content_dir
        ),
        Ok(n) => eprintln!("phoenix-host: preloaded {n} entity templates"),
        Err(e) => eprintln!("phoenix-host: template preload failed: {e}"),
    }

    let server = match HostServer::bind(&args) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("phoenix-host: {e}");
            std::process::exit(1);
        }
    };

    let content = server.content();
    eprintln!(
        "phoenix-host: serving {} — content {:?} epoch {}, protocol {}",
        content.manifest.manifest_path,
        content.manifest.stamp.content_id,
        content.manifest.stamp.content_epoch,
        content.manifest.stamp.protocol,
    );
    for scenario in &content.manifest.scenarios {
        let id = scenario
            .get("id")
            .and_then(project_phoenix::delivery::payload::PayloadValue::as_text)
            .unwrap_or("?");
        eprintln!(
            "phoenix-host:   scenario {id} ({} hulls)",
            scenario.ships().len()
        );
    }
    for finding in &content.findings {
        eprintln!("phoenix-host: manifest finding {finding}");
    }
    match &args.client {
        ClientSource::Bundled { dir } => eprintln!("phoenix-host: client bundle {dir}"),
        ClientSource::Hosted => {
            eprintln!("phoenix-host: no client bundle — manifest endpoints only")
        }
    }

    server.serve_forever(|event| match event {
        HostEvent::Bound { addr } => eprintln!("phoenix-host: listening on http://{addr}/"),
        HostEvent::Served {
            method,
            path,
            status,
        } => eprintln!("phoenix-host: {status} {method} {path}"),
        HostEvent::Refused { path, code } => {
            eprintln!("phoenix-host: 409 {path} ({code})")
        }
        HostEvent::Failed { detail } => eprintln!("phoenix-host: {detail}"),
    });
}
