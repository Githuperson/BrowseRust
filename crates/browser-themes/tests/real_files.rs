//! Opt-in check against a real theme file:
//!   BP_THEME_FILE=/path/to/x.theme.css cargo test -p browser-themes --test real_files -- --nocapture

#[test]
fn parse_the_file_named_by_bp_theme_file() {
    let Ok(path) = std::env::var("BP_THEME_FILE") else {
        eprintln!("BP_THEME_FILE not set; skipping");
        return;
    };
    let src = std::fs::read_to_string(&path).expect("readable theme file");
    let t = browser_themes::parse_theme(&src).expect("parses");
    println!("name={:?} author={:?} version={:?}", t.name, t.author, t.version);
    println!("colors={:#?}", t.colors);
    for w in &t.warnings {
        println!("warning: {w}");
    }
    assert!(t.colors.is_some(), "no usable colours in {path}");
}
