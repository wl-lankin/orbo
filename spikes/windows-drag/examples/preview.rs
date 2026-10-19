// Renders the ring to PNG on any OS to check layout: cargo run --example preview --target <host>
#[allow(dead_code)]
mod actions {
    #[derive(Clone, Copy)]
    pub struct Action { pub title: &'static str, pub sub: &'static str, pub color: [u8; 3], pub ext: &'static str, pub op: () }
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Style { Classic, Glass }
#[path = "../src/render.rs"]
mod render;

fn main() {
    use actions::Action;
    let img = [177, 151, 252];
    let list = [
        Action { title: "JPG", sub: "überall kompatibel", color: img, ext: "jpg", op: () },
        Action { title: "1600 px", sub: "für Mail", color: img, ext: "jpg", op: () },
        Action { title: "BMP", sub: "unkomprimiert", color: img, ext: "bmp", op: () },
        Action { title: "TIFF", sub: "für Druck", color: img, ext: "tiff", op: () },
        Action { title: "GIF", sub: "256 Farben", color: img, ext: "gif", op: () },
        Action { title: "ZIP", sub: "packen", color: [255, 184, 120], ext: "zip", op: () },
    ];
    let fonts = render::Fonts::load().expect("fonts");
    let out = std::env::args().nth(1).unwrap();
    for (style, name) in [(Style::Classic, "classic"), (Style::Glass, "glass")] {
        for (hot, h) in [(None, "idle"), (Some(1), "hot1"), (Some(-1), "center")] {
            let m = render::RingModel { actions: &list, hot, kind: "Bild", count: 1, style };
            let mut bg = tiny_skia::Pixmap::new(840, 840).unwrap();
            bg.fill(tiny_skia::Color::from_rgba8(40, 44, 80, 255));
            let ring = render::ring(840, 2.0, &m, &fonts);
            bg.draw_pixmap(0, 0, ring.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
            bg.save_png(format!("{out}/win-{name}-{h}.png")).unwrap();
        }
    }
}
