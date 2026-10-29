fn main() {
    println!("cargo:rerun-if-changed=catalogue.json");
    let values:std::collections::BTreeMap<String,String>=serde_json::from_str(&std::fs::read_to_string("catalogue.json").unwrap()).unwrap();
    let mut source=String::from("pub const VALUES:&[(&str,&str)]=&[\n");
    for (chinese,english) in values {source.push_str(&format!("({chinese:?},{english:?}),\n"));}
    source.push_str("];\n");
    std::fs::write(std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("catalogue.rs"),source).unwrap();
}
