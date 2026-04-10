use std::{env, fs, process};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: decompress-helper <input_iog1> <output_fctx> [clientpatcher_path]");
        process::exit(1);
    }

    // If a ClientPatcher.exe path is provided (arg 3), tell the native pipeline where to find it
    if args.len() >= 4 && !args[3].is_empty() {
        redux_standalone::encoder_native::set_pe_path(&args[3]);
    }

    let iog1_data = match fs::read(&args[1]) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Read error: {e}");
            process::exit(1);
        }
    };

    let fctx = match redux_standalone::redux::decompress_iog1(&iog1_data) {
        Ok(fctx) => fctx,
        Err(e) => {
            eprintln!("Decompress error: {e}");
            process::exit(1);
        }
    };

    if let Err(e) = fs::write(&args[2], &fctx) {
        eprintln!("Write error: {e}");
        process::exit(1);
    }
}
