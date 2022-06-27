use std::fs;
use std::path::Path;

#[test]
fn test_from_path() {
    let path = Path::new("data");
    let fs = libclient::fs::Filesystem::from_path(path).unwrap();

    fs::write(Path::new("output.txt"), format!("{:#?}", fs)).unwrap();

    let mut header = std::fs::File::create(Path::new("data.sah")).unwrap();
    let mut data = std::fs::File::create(Path::new("data.saf")).unwrap();

    fs.build_with_destination(&mut header, &mut data).unwrap();
}
