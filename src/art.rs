//! The menu's pictures, from two icon libraries rather than drawn by hand:
//!
//! | set    | library                      | used for                          |
//! |--------|------------------------------|-----------------------------------|
//! | tech   | Devicon (MIT), full colour   | the logo beside each port         |
//! | glyph  | Lucide (ISC), line art       | the actions and facts in submenus |
//!
//! `assets/icons/make.sh` rasterises both to 32 px PNGs, which are compiled
//! in; decoding uses the `png` crate the tray library already brings along.

/// Straight-alpha RGBA and its edge, ready for `muda::Icon::from_rgba`.
pub type Rgba = (Vec<u8>, u32);

macro_rules! table {
    ($dir:literal: $($name:literal)*) => {
        &[$(($name, include_bytes!(concat!("../assets/icons/", $dir, "/", $name, ".png")))),*]
    };
}

const TECH: &[(&str, &[u8])] = table!("tech":
    "angular" "apache" "apachekafka" "apollographql" "astro" "bun" "cassandra"
    "clickhouse" "clojure" "cloudflare" "consul" "couchdb" "crystal" "dart"
    "denojs" "django" "docker" "dotnetcore" "elasticsearch" "electron" "elixir"
    "ember" "erlang" "expo" "express" "fastapi" "fastify" "firebase" "flask"
    "flutter" "gatsby" "go" "gradle" "grafana" "haskell" "hugo" "java"
    "jekyll" "jenkins" "julia" "jupyter" "kotlin" "kubernetes" "laravel" "lua"
    "mariadb" "maven" "memcached" "mongodb" "mysql" "neo4j" "nestjs" "netlify"
    "nextjs" "nginx" "ngrok" "nodejs" "nuxtjs" "ocaml" "perl" "phoenix" "php"
    "podman" "postgresql" "prisma" "prometheus" "python" "qwik" "r" "rabbitmq"
    "rails" "react" "redis" "remix" "rollup" "ruby" "rust" "scala" "solidjs"
    "spring" "storybook" "streamlit" "supabase" "svelte" "swift" "symfony"
    "tomcat" "vault" "vercel" "vitejs" "vitest" "vuejs" "webpack" "zig"
);

const GLYPHS: &[(&str, &[u8])] = table!("glyph":
    "circle-stop" "clock" "copy" "file-cog" "folder-open" "globe" "house"
    "octagon-x" "power" "refresh-cw" "terminal" "wifi"
);

pub fn tech_png(key: &str) -> Option<&'static [u8]> {
    TECH.iter().find(|(k, _)| *k == key).map(|(_, png)| *png)
}

/// A technology's logo. On a dark menu, logos drawn mostly in black (Next.js,
/// Rust, Express...) would vanish, so their dark greys are turned light.
pub fn tech(key: &str, dark_menu: bool) -> Option<Rgba> {
    let (mut px, edge) = decode(tech_png(key)?)?;
    if dark_menu && mostly_dark(&px) {
        lighten(&mut px);
    }
    Some((px, edge))
}

/// The colour a glyph is drawn in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ink {
    /// The menu's own text colour, more or less.
    Text,
    /// The one action with a consequence.
    Danger,
    /// Something reachable from the network.
    Accent,
}

/// A Lucide glyph in `ink`, for a menu that is or isn't dark.
pub fn glyph(name: &str, ink: Ink, dark_menu: bool) -> Option<Rgba> {
    let png = GLYPHS.iter().find(|(k, _)| *k == name)?.1;
    let (mut px, edge) = decode(png)?;
    let rgb = match (ink, dark_menu) {
        (Ink::Text, false) => [0x3a, 0x3a, 0x3c],
        (Ink::Text, true) => [0xe5, 0xe5, 0xea],
        (Ink::Danger, false) => [0xd7, 0x2c, 0x22],
        (Ink::Danger, true) => [0xff, 0x5f, 0x57],
        (Ink::Accent, false) => [0x00, 0x6e, 0xdb],
        (Ink::Accent, true) => [0x40, 0x9c, 0xff],
    };
    for p in px.chunks_mut(4) {
        p[..3].copy_from_slice(&rgb);
    }
    Some((px, edge))
}

fn decode(png_bytes: &[u8]) -> Option<Rgba> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.width != info.height || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    let px = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((px, info.width))
}

fn luma(p: &[u8]) -> f32 {
    0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32
}

/// Black, white or a grey, give or take a tint (Django's green is nearly
/// black).
fn greyish(p: &[u8]) -> bool {
    p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) < 40
}

/// At least a quarter of the logo's ink is black or dark grey.
fn mostly_dark(px: &[u8]) -> bool {
    let (mut ink, mut dark) = (0u32, 0u32);
    for p in px.chunks(4).filter(|p| p[3] > 96) {
        ink += 1;
        if greyish(p) && luma(p) < 70.0 {
            dark += 1;
        }
    }
    ink > 0 && dark * 4 >= ink
}

/// For a dark menu, greys swap light for dark: Next.js' black disc with a
/// white N becomes a white disc with a black N. Colours are left alone.
fn lighten(px: &mut [u8]) {
    for p in px.chunks_mut(4).filter(|p| greyish(p)) {
        let v = 255 - luma(p).round() as u8;
        p[..3].fill(v);
    }
}

/// `porta --dump-art <dir>`: every logo on a light and a dark menu, as raw
/// RGBA, for a look at what the dark-menu treatment did.
pub fn dump(dir: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (key, _) in TECH {
        for (theme, dark) in [("light", false), ("dark", true)] {
            if let Some((px, edge)) = tech(key, dark) {
                std::fs::write(format!("{dir}/{key}-{theme}-{edge}.rgba"), px)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_picture_decodes() {
        for (key, _) in TECH {
            let (px, edge) = tech(key, true).unwrap_or_else(|| panic!("{key}"));
            assert_eq!(px.len(), (edge * edge * 4) as usize, "{key}");
        }
        for (name, _) in GLYPHS {
            assert!(glyph(name, Ink::Text, false).is_some(), "{name}");
        }
    }

    #[test]
    fn black_logos_turn_light_on_dark_menus() {
        let ink = |px: &[u8]| -> f32 {
            let opaque: Vec<_> = px.chunks(4).filter(|p| p[3] > 96).collect();
            opaque.iter().map(|p| luma(p)).sum::<f32>() / opaque.len() as f32
        };
        let (light, _) = tech("rust", false).unwrap();
        let (dark, _) = tech("rust", true).unwrap();
        assert!(ink(&dark) > ink(&light) + 100.0);
        // Colourful logos are left as they are.
        assert_eq!(tech("python", true), tech("python", false));
    }
}
