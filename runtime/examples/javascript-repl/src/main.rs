use javascript_repl::{JavaScriptRepl, ReplError, TerminalPermissionPrompt};
use std::io::{self, IsTerminal, Read, Write};
use std::sync::Arc;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), ReplError> {
    if !io::stdin().is_terminal() {
        return run_batch();
    }
    run_interactive()
}

fn run_batch() -> Result<(), ReplError> {
    let mut source = String::new();
    io::stdin().read_to_string(&mut source)?;
    let mut repl = JavaScriptRepl::new(Arc::new(TerminalPermissionPrompt))?;
    let evaluation = repl.evaluate_batch(&source);
    let shutdown = repl.shutdown();
    evaluation?;
    shutdown?;
    Ok(())
}

fn run_interactive() -> Result<(), ReplError> {
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
    println!("  await file.read({{maxBytes: 65536}}); await file.readString({{maxBytes: 65536}})");
    println!("  await file.stat(); await file.close()");
}
