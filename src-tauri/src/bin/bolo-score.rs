//! Score a folder of debug traces or scripted recordings.
//! See `measure::score_main`. Audio is sent to Groq only with `--api`
//! (or `BOLO_SCORE_AUDIO=1`) and `GROQ_API_KEY`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = bolo_lib::measure::score_main(&args) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
