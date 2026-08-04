use javascript_repl::{JavaScriptRepl, ReplError, TerminalPermissionPrompt};
use std::io::{self, Write};
use std::sync::Arc;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), ReplError> {
    let mut repl = JavaScriptRepl::new(Arc::new(TerminalPermissionPrompt))?;
    println!("Zintl Rust JavaScript REPL");
    println!("Type .help for host APIs and .exit to quit.");
    let mut line = String::new();
    loop {
        print!("js> ");
        io::stdout().flush()?;
        line.clear();
        if io::stdin().read_line(&mut line)? == 0 {
            break;
        }
        let source = line.trim();
        match source {
            "" => continue,
            ".exit" | ".quit" => break,
            ".help" => {
                print_help();
                continue;
            }
            _ => {}
        }
        match repl.evaluate(source) {
            Ok(value) => println!("{value}"),
            Err(error) => eprintln!("Uncaught {error}"),
        }
    }
    repl.shutdown()?;
    println!("Runtime shut down.");
    Ok(())
}

fn print_help() {
    println!("Host APIs:");
    println!("  await Zintl.sleep(milliseconds)");
    println!("  await Zintl.invoke('dev.zintl.echo', new Uint8Array([1, 2]))");
    println!("  const dir = await Zintl.requestDirectory('/absolute/path', {{read:true}})");
    println!("  const file = await dir.openRelative('file.txt', {{read:true}})");
    println!("  await file.read({{maxBytes: 65536}}); await file.stat(); await file.close()");
}
