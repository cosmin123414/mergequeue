//! Dump the procedurally-generated sprite sheet to a PNG file.
//!
//! ```sh
//! cargo run --release --example dump_sprite_sheet [path]
//! ```
//!
//! Defaults to `/tmp/mergesmith-sprite.png`. Useful when tuning the
//! silhouette in `src/tui/sprite/placeholder.rs` — open the result in
//! a viewer to see exactly what gets transmitted to the terminal.

use mergesmith::tui::sprite::placeholder::SpriteSheet;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/mergesmith-sprite.png".into());
    let sheet = SpriteSheet::generate();
    std::fs::write(&path, sheet.png_bytes()).expect("write png");
    println!("wrote {} ({} bytes)", path, sheet.png_bytes().len());
}
