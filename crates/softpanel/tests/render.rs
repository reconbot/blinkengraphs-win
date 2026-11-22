use softpanel::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};

#[test]
fn draws_a_label_with_the_font_atlas() {
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(120.0, 40.0),
        )),
        ..Default::default()
    };
    let out = ctx.run_ui(input, |ui| {
        ui.label("blinkengraphs");
    });
    let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
    let mut textures = out.textures_delta;
    let (w, h) = (120, 40);
    let mut px = vec![[0u8; 4]; w * h];
    EguiSoftwareRender::new(ColorFieldOrder::Bgra).render(
        &mut BufferMutRef::new(&mut px, w, h),
        &prims,
        &textures,
        out.pixels_per_point,
    );
    textures.clear();
    let lit = px.iter().filter(|p| p[..3] != [0, 0, 0]).count();
    assert!(lit > 50, "only {lit} pixels drawn");
}
