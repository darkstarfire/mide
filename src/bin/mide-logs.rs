use std::{fs::File, io::{self, BufRead, BufReader}};

use mide::logging;

fn main() -> io::Result<()> {
    let _logging = logging::init("mide-logs", "mide_logs=info");
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mode = match args.as_slice() {
        [] => ("tail", None),
        [arg] if arg == "--all" => ("all", None),
        [arg] if arg == "--errors" => ("errors", None),
        [flag, term] if flag == "--search" => ("search", Some(term.as_str())),
        _ => {
            eprintln!("Uso: mide-logs [--all | --errors | --search TEXTO]");
            std::process::exit(2);
        }
    };
    let path = logging::log_path();
    eprintln!("Log do MIDE: {}", path.display());
    if mode.0 == "tail" {
        for line in logging::tail(300)? {
            println!("{line}");
        }
        return Ok(());
    }

    for file in logging::log_files() {
        let reader = BufReader::new(File::open(file)?);
        for line in reader.lines() {
            let line = line?;
            let lower = line.to_lowercase();
            let show = match mode.0 {
                "all" => true,
                "errors" => lower.contains(" error ") || lower.contains(" warn ") || lower.contains("pânico") || lower.contains("panic") || lower.contains("congel") || lower.contains("watchdog"),
                "search" => lower.contains(&mode.1.unwrap_or_default().to_lowercase()),
                _ => false,
            };
            if show {
                println!("{line}");
            }
        }
    }
    Ok(())
}
