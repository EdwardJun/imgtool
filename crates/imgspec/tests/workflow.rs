//! 文件级流程：解析落盘、语义写回、重新解析保留标注、图标素材（裁剪、改名、AI 网格重绘）、整理层级

use image::DynamicImage;
use imgspec::fixtures::*;
use imgspec::*;

fn write_page(root: &std::path::Path) -> std::path::PathBuf {
    let mut img = canvas(600, 400, [255, 255, 255]);
    let nav = Bounds::new(0, 300, 600, 100);
    rounded_rect(&mut img, nav, 0.0, [247, 248, 250]);
    for k in 0..3u32 {
        icon(&mut img, Bounds::new(80 + k * 200, 320, 40, 40), [22, 119, 255]);
    }
    let card = Bounds::new(40, 40, 520, 200);
    rounded_rect(&mut img, card, 16.0, [22, 119, 255]);
    text_line(&mut img, 80, 80, 32.0, 6, [255, 255, 255], true);
    let path = root.join("ui/home.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    DynamicImage::ImageRgba8(img).save(&path).unwrap();
    path
}

fn icon_nodes(spec: &Spec) -> Vec<Node> {
    let mut v = Vec::new();
    spec.root.walk(&mut |n, _| {
        if n.node_type == NodeType::Icon {
            v.push(n.clone());
        }
    });
    v
}

#[test]
fn analyze_annotate_reanalyze_and_icons() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let image = write_page(root);
    let paths = spec_paths(root, &image, None);
    assert!(paths.spec.ends_with(".imgtool/specs/ui/home/home.spec.json"));
    assert!(paths.overlay.ends_with(".imgtool/specs/ui/home/home.spec.png"));
    assert!(paths.icons_dir.ends_with(".imgtool/specs/ui/home/icons"));

    let r = analyze_file(&paths, &RunOptions::default()).unwrap();
    assert_eq!(r.spec_path, ".imgtool/specs/ui/home/home.spec.json");
    assert!(paths.overlay.is_file());
    assert_eq!(r.spec.source.path, "ui/home.png");
    let icons = icon_nodes(&r.spec);
    assert_eq!(icons.len(), 3, "{}", outline::outline(&r.spec, 100));
    assert_eq!(r.icons.len(), 3);
    for f in &r.icons {
        let p = root.join(&f.path);
        let img = image::open(&p).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (256, 256));
        assert_eq!(img.get_pixel(0, 0)[3], 0, "透明底");
        assert!(f.path.starts_with(".imgtool/specs/ui/home/icons/icon-n"), "{}", f.path);
    }

    // 语义写回：未知编号整体拒绝
    let mut spec = store::load(&paths.spec).unwrap();
    let before = spec.clone();
    let bad = vec![Annotation { id: icons[0].id.clone(), name: Some("home".into()), ..Default::default() }, Annotation { id: "n999".into(), ..Default::default() }];
    let err = store::apply_annotations(&mut spec, &bad, NodeSource::Agent).unwrap_err().to_string();
    assert!(err.contains("n999"), "{err}");
    assert_eq!(spec, before);

    let num = icons[0].id.trim_start_matches('n').to_string();
    let anns = vec![
        Annotation { id: num, name: Some("home".into()), role: Some("tab_icon".into()), ..Default::default() },
        Annotation { id: icons[1].id.clone(), name: Some("home".into()), ..Default::default() },
    ];
    store::apply_annotations(&mut spec, &anns, NodeSource::Agent).unwrap();
    assert!(spec.annotated);
    let renamed = icons::rename_assets(&mut spec, &paths).unwrap();
    assert_eq!(renamed.len(), 2);
    let a0 = spec.root.find(&icons[0].id).unwrap().asset.clone().unwrap();
    let a1 = spec.root.find(&icons[1].id).unwrap().asset.clone().unwrap();
    assert!(a0.path.ends_with("/home.png"), "{}", a0.path);
    assert!(a1.path.ends_with(&format!("/home-{}.png", icons[1].id)), "重名加编号: {}", a1.path);
    assert!(root.join(&a0.path).is_file() && root.join(&a1.path).is_file());
    store::save(&spec, &paths.spec).unwrap();

    // 重新解析：保留标注与素材名，清理旧的 icon-nX 文件
    let again = analyze_file(&paths, &RunOptions::default()).unwrap();
    assert!(again.kept_annotations >= 2);
    let n0 = again.spec.root.find(&icons[0].id).and_then(|n| n.name.clone());
    let named: Vec<String> = icon_nodes(&again.spec).iter().filter_map(|n| n.name.clone()).collect();
    assert_eq!(named.len(), 2, "{n0:?}");
    let files: Vec<String> = std::fs::read_dir(&paths.icons_dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    assert_eq!(files.len(), 3, "{files:?}");
    assert!(files.iter().any(|f| f == "home.png"));

    // AI 网格重绘：计划 + 参考图 + 拆分
    let mut spec = store::load(&paths.spec).unwrap();
    let plan = icons::plan_icon_sheet(&spec, None, Some("线性图标，2px 线宽")).unwrap();
    assert_eq!((plan.cols, plan.rows), (2, 2));
    assert!(plan.prompt.contains("home") && plan.prompt.contains("最后一行只有 1 个图标") && plan.prompt.contains("2px"), "{}", plan.prompt);
    let refimg = image::load_from_memory(&icons::reference_sheet(&image::open(&image).unwrap(), &spec, &plan).unwrap()).unwrap();
    assert_eq!((refimg.width(), refimg.height()), (512, 512));

    let mut sheet = canvas(1024, 1024, [255, 255, 255]);
    for i in 0..3u32 {
        let (c, r) = (i % 2, i / 2);
        icon(&mut sheet, Bounds::new(c * 512 + 156, r * 512 + 156, 200, 200), [22, 119, 255]);
    }
    let (files, warnings) = icons::apply_icon_sheet(&DynamicImage::ImageRgba8(sheet), &mut spec, &plan, &paths, &IconOptions::default()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(files.len(), 3);
    assert!(files.iter().all(|f| f.source == AssetSource::Generated));
    let home = spec.root.find(&plan.ids[0]).unwrap().asset.clone().unwrap();
    assert_eq!(home.source, AssetSource::Generated);
    store::save(&spec, &paths.spec).unwrap();

    // 再次解析不会用裁剪图覆盖 AI 高清版
    let third = analyze_file(&paths, &RunOptions::default()).unwrap();
    let gen = third.icons.iter().filter(|f| f.source == AssetSource::Generated).count();
    assert_eq!(gen, 3);
}

#[test]
fn custom_icons_dir_from_preset() {
    let dir = tempfile::tempdir().unwrap();
    let image = write_page(dir.path());
    let paths = spec_paths(dir.path(), &image, Some("src/assets/icons"));
    let r = analyze_file(&paths, &RunOptions::default()).unwrap();
    assert!(r.icons.iter().all(|f| f.path.starts_with("src/assets/icons/")), "{:?}", r.icons);
    // 已有同名的别的文件：不覆盖
    std::fs::write(dir.path().join("src/assets/icons/keep.png"), b"x").unwrap();
    let mut spec = store::load(&paths.spec).unwrap();
    let id = icon_nodes(&spec)[0].id.clone();
    store::apply_annotations(&mut spec, &[Annotation { id: id.clone(), name: Some("keep".into()), ..Default::default() }], NodeSource::Manual).unwrap();
    icons::rename_assets(&mut spec, &paths).unwrap();
    assert_eq!(std::fs::read(dir.path().join("src/assets/icons/keep.png")).unwrap(), b"x");
    assert!(spec.root.find(&id).unwrap().asset.as_ref().unwrap().path.contains("keep-"));
}

#[test]
fn normalize_after_edit_keeps_ids() {
    let dir = tempfile::tempdir().unwrap();
    let image = write_page(dir.path());
    let img = image::open(&image).unwrap();
    let mut spec = analyze(&img, "ui/home.png", &AnalyzeOptions::default());
    let ids_before: Vec<String> = {
        let mut v = Vec::new();
        spec.root.walk(&mut |n, _| {
            if !n.is_plain_group() {
                v.push(n.id.clone());
            }
        });
        v
    };
    // 把第一个图标挪进蓝色卡片里
    let icon_id = icon_nodes(&spec)[0].id.clone();
    let n = spec.root.find_mut(&icon_id).unwrap();
    n.bounds = Bounds::new(480, 60, 40, 40);
    layout::normalize(&mut spec);
    let mut ids_after = Vec::new();
    spec.root.walk(&mut |n, _| {
        if !n.is_plain_group() {
            ids_after.push(n.id.clone());
        }
    });
    for id in &ids_before {
        assert!(ids_after.contains(id), "编号 {id} 丢失");
    }
    // 图标现在属于卡片
    let mut parent_of_icon = None;
    spec.root.walk(&mut |n, _| {
        if n.children.iter().any(|c| c.id == icon_id) || n.children.iter().any(|c| c.is_plain_group() && c.children.iter().any(|g| g.id == icon_id)) {
            parent_of_icon = Some(n.clone());
        }
    });
    let p = parent_of_icon.expect("应找到父节点");
    assert!(p.style.fill.is_some() || p.is_plain_group(), "{}", outline::outline(&spec, 50));
    let card = spec.root.find(&p.id).unwrap();
    assert!(card.bounds.contains_ratio(&Bounds::new(480, 60, 40, 40)) > 0.9);
    // 新节点的编号唯一
    let mut all = Vec::new();
    spec.root.walk(&mut |n, _| all.push(n.id.clone()));
    let uniq: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(uniq.len(), all.len());
}

#[test]
fn analyze_region_refreshes_style() {
    let mut img = canvas(300, 200, [255, 255, 255]);
    rounded_rect(&mut img, Bounds::new(50, 50, 200, 80), 12.0, [255, 77, 79]);
    let img = DynamicImage::ImageRgba8(img);
    let n = analyze_region(&img, Bounds::new(46, 46, 210, 90)).unwrap();
    assert!(n.bounds.iou(&Bounds::new(50, 50, 200, 80)) > 0.95, "{:?}", n.bounds);
    assert_eq!(n.style.fill.as_ref().unwrap().main_color(), Some([255, 77, 79]));
    assert_eq!(n.style.corner_radius, Some(CornerRadius::Uniform(12)));
}

#[test]
fn redraw_skips_tiny_icons_and_adds_context() {
    let mut img = canvas(600, 400, [255, 255, 255]);
    icon(&mut img, Bounds::new(60, 60, 44, 44), [22, 119, 255]);
    text_line(&mut img, 120, 70, 26.0, 4, [30, 30, 30], false);
    icon(&mut img, Bounds::new(60, 200, 16, 16), [22, 119, 255]);
    text_line(&mut img, 90, 200, 20.0, 6, [30, 30, 30], false);
    let img = DynamicImage::ImageRgba8(img);
    let mut spec = analyze(&img, "x.png", &AnalyzeOptions { mode: Some(SpecKind::Page), ..Default::default() });
    let icons = icon_nodes(&spec);
    assert_eq!(icons.len(), 2, "{}", outline::outline(&spec, 40));
    let (big, small) = if icons[0].bounds.width > icons[1].bounds.width { (&icons[0], &icons[1]) } else { (&icons[1], &icons[0]) };

    // 默认：小图标被跳过并说明原因
    let plan = icons::plan_icon_sheet(&spec, None, None).unwrap();
    assert_eq!(plan.ids, vec![big.id.clone()]);
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].id, small.id);
    assert_eq!(plan.unnamed, vec![big.id.clone()], "没有命名的图标要列出来");
    assert!(plan.prompt.contains("第二张参考图"));

    // 旁边已标注的文字写进提示词
    let mut text_id = String::new();
    spec.root.walk(&mut |n, _| {
        if n.node_type == NodeType::Text && n.bounds.y < 120 && text_id.is_empty() {
            text_id = n.id.clone();
        }
    });
    store::apply_annotations(&mut spec, &[Annotation { id: text_id, text: Some("申报时间".into()), ..Default::default() }], NodeSource::Agent).unwrap();
    let plan = icons::plan_icon_sheet(&spec, None, None).unwrap();
    assert!(plan.prompt.contains("旁边的文字是「申报时间」"), "{}", plan.prompt);

    // 显式指定：强制加入，并标出原图太小
    let forced = icons::plan_icon_sheet(&spec, Some(&[big.id.clone(), small.id.clone()]), None).unwrap();
    assert_eq!(forced.ids.len(), 2);
    assert!(forced.skipped.is_empty());
    assert_eq!(forced.low_res, vec![small.id.clone()]);

    // 只剩小图标时报错而不是白花一次额度
    let only_small = analyze(
        &DynamicImage::ImageRgba8({
            let mut i = canvas(600, 400, [255, 255, 255]);
            icon(&mut i, Bounds::new(60, 60, 16, 16), [22, 119, 255]);
            i
        }),
        "y.png",
        &AnalyzeOptions { mode: Some(SpecKind::Page), ..Default::default() },
    );
    assert!(icons::plan_icon_sheet(&only_small, None, None).unwrap_err().to_string().contains("太小"));

    // 上下文参考图：完整设计稿（长边 ≤ 1536）
    let ctx = image::load_from_memory(&icons::context_sheet(&img, &spec, &forced).unwrap()).unwrap();
    assert_eq!((ctx.width(), ctx.height()), (600, 400));
    let red = ctx.to_rgba8().pixels().filter(|p| p[0] > 230 && p[1] < 80 && p[2] < 80).count();
    assert!(red > 100, "应画出红框");
}
