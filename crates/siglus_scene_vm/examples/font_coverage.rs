//! Check actual glyph availability, including punctuation, in an explicit font.
use ab_glyph::{Font, FontArc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: font_coverage FONT CODEPOINTS...")?;
    let font = FontArc::try_from_vec(std::fs::read(path)?)?;
    for path in args {
        let mut missing = Vec::new();
        let mut count = 0;
        for line in std::fs::read_to_string(&path)?.lines() {
            let c = char::from_u32(u32::from_str_radix(line, 16)?).ok_or("invalid codepoint")?;
            if c.is_control() || ('\u{e000}'..='\u{f8ff}').contains(&c) {
                continue;
            }
            count += 1;
            if font.glyph_id(c).0 == 0 {
                missing.push(format!("U+{:04X}", c as u32));
            }
        }
        println!("{path}: checked={count} missing={missing:?}");
    }
    Ok(())
}
