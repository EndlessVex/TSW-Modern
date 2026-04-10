//! 32-bit IOg1 decompression helper for bit-exact texture processing.
//!
//! Usage: decompress-helper.exe <input_path> <output_path>
//! Exit codes: 0 = success, 1 = error (message on stderr)

use std::{env, fs, process};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: decompress-helper <input_iog1> <output_fctx>");
        process::exit(1);
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
