//! Writes the drawn window icon as a PNG for the Linux package.
//!
//! `just package` puts this file beside the binary as the desktop entry's
//! icon. It is generated rather than committed, so the tree keeps no binary
//! asset and the menu's icon cannot drift from the one the window shows
//! (ADRs 058, 077).
//!
//!     cargo run -p winter --example write-icon -- <path/to/winter.png>

/// The size a desktop entry's icon is installed at.
const SIZE: usize = 256;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let destination = std::env::args()
        .nth(1)
        .ok_or("usage: write-icon <path/to/winter.png>")?;

    let file = std::fs::File::create(&destination)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), SIZE as u32, SIZE as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&winter_icon::rgba(SIZE))?;

    println!("wrote {destination}");
    Ok(())
}
