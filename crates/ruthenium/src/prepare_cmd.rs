//! `ruthenium locate` and `ruthenium prepare`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use ruthenium_prepare::discover::{self, Candidate, Env, Launcher};
use ruthenium_prepare::{datagen, import, java, supported_version};

#[derive(Debug, Default)]
struct Options {
    from: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    java: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut opts = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--from" => opts.from = Some(value()?),
            "--data-dir" => opts.data_dir = Some(value()?),
            "--java" => opts.java = Some(value()?),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(opts)
}

fn candidates(opts: &Options, env: &Env) -> Vec<Candidate> {
    let version = supported_version();
    match &opts.from {
        Some(folder) => discover::search_folder(folder, version),
        None => discover::search(&discover::default_roots(env), version),
    }
}

fn print_candidates(candidates: &[Candidate]) {
    for c in candidates {
        match &c.outcome {
            Ok(install) => println!(
                "  ok        {:<18} {} (client jar: {})",
                c.launcher.name(),
                c.root.display(),
                install
                    .client_jar
                    .strip_prefix(&c.root)
                    .unwrap_or(&install.client_jar)
                    .display()
            ),
            Err(why) => println!(
                "  {:<9} {:<18} {} ({why})",
                "-",
                c.launcher.name(),
                c.root.display()
            ),
        }
    }
}

fn not_found(opts: &Options, candidates: &[Candidate]) -> ExitCode {
    let id = supported_version().id;
    let names: Vec<&str> = Launcher::ALL.iter().map(|l| l.name()).collect();
    eprintln!(
        "No verified Minecraft {id} install found.\nPlay {id} once in your launcher ({}), then run this again.",
        names.join(", ")
    );
    if candidates.is_empty() {
        if let Some(from) = &opts.from {
            eprintln!("{} does not look like a launcher folder.", from.display());
        }
    } else {
        eprintln!("Searched:");
        print_candidates(candidates);
    }
    eprintln!("If your launcher keeps its files elsewhere, pass --from <folder>.");
    ExitCode::FAILURE
}

pub fn locate(args: &[String]) -> ExitCode {
    let opts = match parse(args) {
        Ok(o) => o,
        Err(e) => return usage_error(&e),
    };
    let found = candidates(&opts, &Env::from_process());
    if discover::first_install(&found).is_none() {
        return not_found(&opts, &found);
    }
    println!("Minecraft {}:", supported_version().id);
    print_candidates(&found);
    ExitCode::SUCCESS
}

pub fn prepare(args: &[String]) -> ExitCode {
    let opts = match parse(args) {
        Ok(o) => o,
        Err(e) => return usage_error(&e),
    };
    let env = Env::from_process();
    let version = supported_version();
    let Some(data_dir) = opts
        .data_dir
        .clone()
        .or_else(|| ruthenium_prepare::default_data_dir(&env))
    else {
        eprintln!("cannot work out a data folder; pass --data-dir <folder>");
        return ExitCode::FAILURE;
    };
    let cache = data_dir.join("game");

    let found = candidates(&opts, &env);
    let Some(install) = discover::first_install(&found) else {
        return not_found(&opts, &found);
    };
    println!(
        "Using {} at {}",
        install.launcher.name(),
        install.root.display()
    );

    let start = Instant::now();
    let imported = match import::open(version, &cache) {
        Some(imported) => {
            println!("Import: up to date ({})", imported.dir.display());
            imported
        }
        None => match import::import(install, version, &cache) {
            Ok((imported, s)) => {
                println!(
                    "Import: {} asset objects; {} files linked, {} copied, {} already present ({:.1?})",
                    s.objects,
                    s.linked,
                    s.copied,
                    s.present,
                    start.elapsed()
                );
                imported
            }
            Err(e) => {
                eprintln!("Import failed: {e}");
                return ExitCode::FAILURE;
            }
        },
    };

    if datagen::is_current(&imported) {
        println!("Data generator: up to date");
        println!("Reports: {}", datagen::reports_dir(&imported).display());
        return ExitCode::SUCCESS;
    }

    let java = match java::find_java(Some(install), &version.java, opts.java.as_deref()) {
        Ok(j) => j,
        Err(e) => {
            eprintln!(
                "{e}\nInstall a Java {}+ runtime or pass --java <path>.",
                version.java.major
            );
            return ExitCode::FAILURE;
        }
    };
    println!("Java: {} ({})", java.description, java.path.display());

    let start = Instant::now();
    match datagen::run_reports(&java, &imported) {
        Ok((reports, ran)) => {
            if ran {
                println!("Data generator: done ({:.1?})", start.elapsed());
            } else {
                println!("Data generator: up to date");
            }
            println!("Reports: {}", reports.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Data generator failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage_error(msg: &str) -> ExitCode {
    eprintln!("{msg}\n\n{}", crate::USAGE);
    ExitCode::FAILURE
}
