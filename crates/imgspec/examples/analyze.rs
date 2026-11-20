//! 开发用：解析一张图并打印精简树。
//! cargo run -p imgspec --example analyze -- <图片> [--out <目录>] [--json]

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("用法: analyze <图片> [--out <目录>] [--json]");
        std::process::exit(2);
    };
    let path = PathBuf::from(path);
    let mut img = imgspec::decode_file(&path).expect("读取图片失败");
    // --crop x,y,w,h：只分析一块区域（排查用）
    if let Some(i) = args.iter().position(|a| a == "--crop") {
        let v: Vec<u32> = args[i + 1].split(',').map(|s| s.parse().expect("--crop x,y,w,h")).collect();
        img = img.crop_imm(v[0], v[1], v[2], v[3]);
    }
    let t = std::time::Instant::now();
    let spec = imgspec::analyze(&img, &path.display().to_string(), &Default::default());
    let ms = t.elapsed().as_millis();
    if args.iter().any(|a| a == "--json") {
        println!("{}", serde_json::to_string_pretty(&spec).unwrap());
    } else {
        println!("{}", imgspec::outline::outline(&spec, 400));
    }
    eprintln!("{} 个节点，用时 {ms}ms", spec.root.count());
    if let Some(i) = args.iter().position(|a| a == "--out") {
        let dir = PathBuf::from(args.get(i + 1).expect("--out 需要目录"));
        std::fs::create_dir_all(&dir).unwrap();
        let ov = imgspec::overlay::render(&img, &spec, imgspec::OVERLAY_MAX_EDGE);
        let stem = path.file_stem().unwrap().to_string_lossy();
        ov.save(dir.join(format!("{stem}.spec.png"))).unwrap();
        std::fs::write(dir.join(format!("{stem}.spec.json")), serde_json::to_string_pretty(&spec).unwrap()).unwrap();
    }
}
