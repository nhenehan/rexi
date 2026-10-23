//! The rules for which files `xs:include` and `xs:import` may read
//! ([`SchemaLoadOptions`]): under the main schema's folder by default, never a URL,
//! a pipe or a device, and never a file over the size limit.

use std::fs;
use std::path::{Path, PathBuf};

use rexi_schema::{
    SchemaError, SchemaLoadOptions, SchemaRoot, compile_files, compile_files_with, compile_str,
    compile_str_with,
};

/// A fresh, empty folder for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rexi-load-rules-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    dir
}

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::write(path, text).unwrap_or_else(|e| panic!("{e}"));
}

/// A schema that includes `location` and declares one element.
fn including(location: &str) -> String {
    format!(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:include schemaLocation="{location}"/>
  <xs:element name="r" type="T"/>
</xs:schema>"#
    )
}

const TYPES: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:simpleType name="T"><xs:restriction base="xs:string"/></xs:simpleType>
</xs:schema>"#;

fn refused(result: Result<impl Sized, SchemaError>) -> String {
    match result {
        Err(SchemaError::Refused { reason, .. }) => reason,
        Err(other) => panic!("expected Refused, got {other}"),
        Ok(_) => panic!("expected Refused, got Ok"),
    }
}

#[test]
fn includes_under_the_main_folder_work() {
    let dir = scratch("under");
    write(&dir.join("types.xsd"), TYPES);
    write(&dir.join("sub/more.xsd"), TYPES);
    write(&dir.join("main.xsd"), &including("types.xsd"));
    write(&dir.join("main2.xsd"), &including("sub/./more.xsd"));
    assert!(compile_files(&[&dir.join("main.xsd")]).is_ok());
    assert!(compile_files(&[&dir.join("main2.xsd")]).is_ok());
}

#[test]
fn a_ubl_style_layout_needs_a_root() {
    // set/maindoc/order.xsd includes ../common/types.xsd, outside its own folder.
    let set = scratch("ubl");
    write(&set.join("common/types.xsd"), TYPES);
    let main = set.join("maindoc/order.xsd");
    write(&main, &including("../common/types.xsd"));
    let reason = refused(compile_files(&[&main]));
    assert!(reason.contains("set a schema root"), "{reason}");
    let opts = SchemaLoadOptions::default().with_root(SchemaRoot::Folder(set.clone()));
    assert!(compile_files_with(&[&main], &opts).is_ok());
    let opts = SchemaLoadOptions::default().with_root(SchemaRoot::Anywhere);
    assert!(compile_files_with(&[&main], &opts).is_ok());
}

#[test]
fn outside_paths_are_refused_whether_or_not_they_exist() {
    let dir = scratch("outside");
    let elsewhere = scratch("outside-elsewhere");
    write(&elsewhere.join("types.xsd"), TYPES);
    let absolute = elsewhere.join("types.xsd").display().to_string();
    for location in [
        absolute.as_str(),
        "../../../../../../../../etc/passwd",
        "../../../../../../../../no/such/file.xsd",
    ] {
        write(&dir.join("main.xsd"), &including(location));
        let reason = refused(compile_files(&[&dir.join("main.xsd")]));
        assert!(reason.contains("outside"), "{location}: {reason}");
    }
}

#[test]
fn urls_are_never_fetched() {
    let dir = scratch("urls");
    for location in ["http://example.com/types.xsd", "file:///etc/passwd"] {
        write(&dir.join("main.xsd"), &including(location));
        let opts = SchemaLoadOptions::default().with_root(SchemaRoot::Anywhere);
        let reason = refused(compile_files_with(&[&dir.join("main.xsd")], &opts));
        assert!(reason.contains("URL"), "{location}: {reason}");
    }
}

#[test]
fn a_schema_in_memory_includes_only_under_its_base_dir() {
    let dir = scratch("memory");
    write(&dir.join("types.xsd"), TYPES);
    let xsd = including("types.xsd");
    let reason = refused(compile_str(&xsd, None));
    assert!(reason.contains("no base folder"), "{reason}");
    assert!(compile_str(&xsd, Some(&dir)).is_ok());
    let opts = SchemaLoadOptions::default().with_root(SchemaRoot::Folder(dir.clone()));
    let absolute = including(&dir.join("types.xsd").display().to_string());
    assert!(compile_str_with(&absolute, None, &opts).is_ok());
}

#[test]
fn devices_and_big_files_are_refused() {
    let dir = scratch("devices");
    let anywhere = SchemaLoadOptions::default().with_root(SchemaRoot::Anywhere);
    if Path::new("/dev/zero").exists() {
        write(&dir.join("main.xsd"), &including("/dev/zero"));
        let reason = refused(compile_files_with(&[&dir.join("main.xsd")], &anywhere));
        assert!(reason.contains("regular file"), "{reason}");
    }
    write(&dir.join("types.xsd"), TYPES);
    write(&dir.join("main.xsd"), &including("types.xsd"));
    let small = SchemaLoadOptions::default().with_max_file_bytes(Some(TYPES.len() as u64 - 1));
    let reason = refused(compile_files_with(&[&dir.join("main.xsd")], &small));
    assert!(reason.contains("bigger than"), "{reason}");
}

#[cfg(unix)]
#[test]
fn pipes_and_links_out_of_the_root_are_refused() {
    let dir = scratch("unix");
    let elsewhere = scratch("unix-elsewhere");
    write(&elsewhere.join("types.xsd"), TYPES);
    std::os::unix::fs::symlink(elsewhere.join("types.xsd"), dir.join("link.xsd"))
        .unwrap_or_else(|e| panic!("{e}"));
    write(&dir.join("main.xsd"), &including("link.xsd"));
    let reason = refused(compile_files(&[&dir.join("main.xsd")]));
    assert!(reason.contains("outside"), "{reason}");

    // A named pipe would block a reader forever; it must be refused unopened.
    let made = std::process::Command::new("mkfifo")
        .arg(dir.join("pipe.xsd"))
        .status()
        .is_ok_and(|s| s.success());
    if made {
        write(&dir.join("main.xsd"), &including("pipe.xsd"));
        let reason = refused(compile_files(&[&dir.join("main.xsd")]));
        assert!(reason.contains("regular file"), "{reason}");
    }
}
