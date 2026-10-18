//! Inspect original and translated resource pairs without modifying the input.
//! Decoded text and key material are written only to the explicit output folder.
use siglus_assets::gameexe::{GameexeDecodeOptions, decode_gameexe_dat_bytes};
use siglus_assets::key_toml::{StringEncryptionOverride, write_key16_to_file};
use siglus_assets::scene_pck::{CIndex, ScenePck, ScenePckDecodeOptions, SceneStringCodec};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 {
        return Err("usage: inspect_project INPUT_DIRECTORY OUTPUT_DIRECTORY".into());
    }
    let input = Path::new(&args[1]);
    let output = Path::new(&args[2]);
    for (variant, config, scene) in [
        ("original", "Gameexe.dat", "Scene.pck"),
        ("chs", "gameexe.chs", "scene.chs"),
    ] {
        let game = fs::read(input.join(config))?;
        let scene = fs::read(input.join(scene))?;
        let out = output.join(variant);
        fs::create_dir_all(&out)?;
        let recovered = if out.join("key.toml").is_file() {
            siglus_assets::key_toml::load_key16_from_file(&out.join("key.toml"))?
                .ok_or_else(|| "cached key file contains no key".to_owned())
        } else {
            siglus_key_recovery::recover_key_from_resources(&game, &scene)
        };
        let key = match recovered {
            Ok(key) => key,
            Err(error) => {
                fs::write(out.join("recovery-error.txt"), &error)?;
                println!("{variant}\tkey_recovery_failed\t{error}");
                continue;
            }
        };
        write_key16_to_file(&out.join("key.toml"), key)?;
        let options = GameexeDecodeOptions {
            exe_key16: Some(key),
            game_angou_code: Some(siglus_assets::keys::GAMEEXE_KEY.to_vec()),
            ..Default::default()
        };
        let (text, report) = decode_gameexe_dat_bytes(&game, &options)?;
        fs::write(out.join("Gameexe.ini"), &text)?;
        println!(
            "{variant}\tgameexe_ok\t{:?}\t{} chars",
            report.encoding,
            text.chars().count()
        );
        let options = ScenePckDecodeOptions {
            exe_angou_element: Some(key.to_vec()),
            easy_angou_code: Some(siglus_assets::keys::SCENE_KEY.to_vec()),
            string_encryption_override: StringEncryptionOverride::Mdl,
        };
        let pack = ScenePck::load_lazy_from_bytes(scene, &options)?;
        let mut rows = fs::File::create(out.join("scenes.tsv"))?;
        let mut previews = fs::File::create(out.join("text-previews.tsv"))?;
        let mut warnings = fs::File::create(out.join("utf16-warnings.tsv"))?;
        let mut codepoints = BTreeSet::new();
        writeln!(rows, "scene\tname\tbytes\tstrings\tkana\tcjk\treplacement")?;
        for no in 0..pack.header.scn_data_cnt as usize {
            let bytes = pack.scn_data_slice(no)?;
            if bytes.is_empty() {
                continue;
            }
            let read = |at: usize| -> usize {
                u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
            };
            let index = read(12);
            let count = read(16);
            let data = read(20);
            let mut kana = 0;
            let mut cjk = 0;
            let mut replacement = 0;
            let mut selected = 0;
            for id in 0..count {
                let entry = CIndex::read(&bytes, index + id * 8)?;
                if entry.offset < 0 || entry.size < 0 {
                    return Err("negative string range".into());
                }
                let at = data + entry.offset as usize * 2;
                let end = at + entry.size as usize * 2;
                let raw = bytes.get(at..end).ok_or("string range out of bounds")?;
                let mask = if pack.string_codec == SceneStringCodec::Xor {
                    28807u32.wrapping_mul(id as u32) as u16
                } else {
                    0
                };
                let units: Vec<_> = raw
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]) ^ mask)
                    .collect();
                let text = String::from_utf16_lossy(&units);
                if text.contains('\u{fffd}') {
                    writeln!(
                        warnings,
                        "{no}\t{id}\tliteral_fffd={}\tinvalid_utf16={}",
                        units.iter().filter(|u| **u == 0xfffd).count(),
                        String::from_utf16(&units).is_err()
                    )?;
                }
                codepoints.extend(text.chars());
                kana += text
                    .chars()
                    .filter(|c| ('\u{3040}'..='\u{30ff}').contains(c))
                    .count();
                cjk += text
                    .chars()
                    .filter(|c| ('\u{3400}'..='\u{9fff}').contains(c))
                    .count();
                replacement += text.chars().filter(|c| *c == '\u{fffd}').count();
                if selected < 24 && text.chars().any(|c| ('\u{3400}'..='\u{9fff}').contains(&c)) {
                    writeln!(
                        previews,
                        "{no}\t{id}\t{}",
                        text.replace(['\n', '\r', '\t'], " ")
                    )?;
                    selected += 1;
                }
            }
            writeln!(
                rows,
                "{no}\t{}\t{}\t{count}\t{kana}\t{cjk}\t{replacement}",
                pack.find_scene_name(no).unwrap_or(""),
                bytes.len()
            )?;
        }
        let mut glyphs = fs::File::create(out.join("codepoints.tsv"))?;
        for c in codepoints {
            writeln!(glyphs, "{:X}", c as u32)?;
        }
        println!(
            "{variant}\tscenes_ok\t{}\t{:?}",
            pack.header.scn_data_cnt, pack.string_codec
        );
    }
    Ok(())
}
