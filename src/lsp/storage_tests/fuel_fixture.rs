//! Authored non-stock overlay on the public storage fixture. No live capture,
//! getter/loader/producer certification or JAR identity is invented here.
use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs};

const BASE:&str="ch.njol.skript.classes.ClassInfo";
const COMPONENT:&str="net.kyori.adventure.text.Component";
fn fingerprint(parts:&[String])->String {
    parts.iter().map(|p|format!("{}:{p}",p.encode_utf16().count())).collect::<Vec<_>>().join("|")
}
fn words(v:&Value)->Vec<String> {v.as_array().unwrap().iter().map(|v|v.as_str().unwrap().to_owned()).collect()}
fn field(v:&Value,key:&str)->String {v[key].as_str().unwrap().to_owned()}
fn write_json(dir:&Path,name:&str,value:&Value) {fs::write(dir.join(name),serde_json::to_vec(value).unwrap()).unwrap();}
fn read_json(dir:&Path,name:&str)->Value {serde_json::from_slice(&fs::read(dir.join(name)).unwrap()).unwrap()}

// Schema-7's Java-compatible identity projection. ssg::load independently checks
// this result, including UTF-16 lengths and optional server implementation data.
fn refresh_manifest(dir:&Path) {
    let mut m=read_json(dir,"Manifest.json");
    let files=ssg::data_files_for_schema(m["schemaVersion"].as_u64().unwrap() as u32).unwrap();
    let text=files.iter().map(|name|((*name).to_owned(),fs::read_to_string(dir.join(name)).unwrap())).collect::<BTreeMap<_,_>>();
    let mut digest=Sha256::new();
    for (i,(name,json)) in text.iter().enumerate() {
        if i>0 {digest.update(b"|");}
        for part in [name,json] {digest.update(part.encode_utf16().count().to_string().as_bytes());digest.update(b":");digest.update(part.as_bytes());}
    }
    m["contentDigest"]=format!("{:x}",digest.finalize()).into();
    let server=&m["server"];
    let mut server_parts=["name","version","bukkitVersion","minecraftVersion","javaVersion"].map(|k|field(server,k)).to_vec();
    if !server["implementationClass"].is_null()||!server["implementationJarSha256"].is_null() {
        server_parts.extend(["implementationClass","implementationJarSha256"].map(|k|server[k].as_str().unwrap_or_default().to_owned()));
    }
    let plugins=m["plugins"].as_array().unwrap().iter().map(|p|fingerprint(&[
        p["loadOrder"].as_u64().unwrap().to_string(),field(p,"name"),field(p,"version"),field(p,"main"),
        p["enabled"].as_bool().unwrap().to_string(),words(&p["depend"]).join(","),words(&p["softDepend"]).join(","),
        words(&p["loadBefore"]).join(","),p["jarSha256"].as_str().unwrap_or_default().to_owned()
    ])).collect::<Vec<_>>();
    let caps=&m["capabilities"];
    let kinds=["conditions","effects","events","expressions","types","functions","sections","structures",
        "properties","arithmetic","converters","comparators","eventValues"].iter()
        .map(|k|if caps["syntaxKinds"][*k].as_bool().unwrap(){'1'}else{'0'}).collect::<String>();
    let capabilities=fingerprint(&[field(caps,"syntaxApi"),field(caps,"eventValueApi"),kinds,
        format!("{}:{}",u8::from(caps["aliases"]["supported"].as_bool().unwrap()),u8::from(caps["aliases"]["collected"].as_bool().unwrap()))]);
    let mut names=words(&m["files"]);names.sort();
    let encoded=fingerprint(&[m["schemaVersion"].as_u64().unwrap().to_string(),field(&m,"contentDigest"),
        fingerprint(&server_parts),field(&m,"language"),fingerprint(&plugins),capabilities,fingerprint(&names)]);
    m["snapshotId"]=format!("{:x}",Sha256::digest(encoded.as_bytes())).into();
    write_json(dir,"Manifest.json",&m);
}

fn authored_catalog()->(tempfile::TempDir,Arc<Catalog>) {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("parser-wasm/tests/data/type-parser-versions/skript-2.15.4-storage-domain");
    let temp=tempfile::tempdir().unwrap();
    for name in ssg::data_files_for_schema(7).unwrap().iter().copied().chain(["Manifest.json"]) {
        fs::copy(base.join(name),temp.path().join(name)).unwrap();
    }
    let mut types=read_json(temp.path(),"Types.json");let mut hierarchy=read_json(temp.path(),"ClassHierarchy.json");
    assert_eq!(types.as_array().unwrap().len(),131);
    for i in 0..5 {
        let class=format!("fixture.fuel.Value{i}");let id=format!("type:authored:{:x}",Sha256::digest(class.as_bytes()));
        let order=types.as_array().unwrap().len();
        let provider=json!({"name":"AuthoredFuelFixture","version":"1"});
        types.as_array_mut().unwrap().push(json!({"typeParseOrder":order,"name":format!("Fuel fixture {i}"),
            "addon":provider,"definitionId":id,"registrationId":id,"hasDocs":false,"originalClass":class,
            "classType":"Class","codeName":format!("fuelguard{i}"),"superClass":"java.lang.Object","interfaces":[],"assignableTo":["object"],
            "noun":{"key":format!("fixture.fuel{i}"),"singular":format!("fuelguard{i}"),"plural":format!("fuelguard{i}s"),"gender":0,"genderId":"a"},
            "hasParser":false,"hasSerializer":false,"hasSupplier":false,"properties":[],
            "typeRuntime":{"version":3,"classKind":"Class","ancestryComplete":true,"ancestors":["java.lang.Object"],
                "classState":"present","classValue":class,"classInfoClass":BASE,"classGetterDeclaringClass":BASE,
                "cloneMethodState":"known","cloneMethodDeclaringClass":BASE,"clonerGetterDeclaringClass":BASE,"clonerState":"absent",
                "serializeAsGetterDeclaringClass":BASE,"serializeAsState":"absent"}}));
        hierarchy.as_array_mut().unwrap().push(json!({"name":class,"binaryName":class,"kind":"Class","superClass":"java.lang.Object",
            "interfaces":[],"methods":[],"provider":provider}));
    }
    write_json(temp.path(),"Types.json",&types);write_json(temp.path(),"ClassHierarchy.json",&hierarchy);
    let mut m=read_json(temp.path(),"Manifest.json");
    m["plugins"].as_array_mut().unwrap().push(json!({"loadOrder":2,"name":"AuthoredFuelFixture","version":"1",
        "main":"fixture.fuel.Plugin","enabled":true,"depend":[],"softDepend":[],"loadBefore":[]}));
    write_json(temp.path(),"Manifest.json",&m);refresh_manifest(temp.path());
    let catalog=Arc::new(ssg::load(temp.path()).unwrap().into_catalog());
    (temp,catalog)
}

#[test]
fn large_mixed_declared_property_storage_fits_default_fuel() {
    let (_files,catalog)=authored_catalog();
    for expression in ["name of player","player's name"] {
        let config=HostConfig {syntax_catalog:Some(Arc::clone(&catalog)),
            runtime_profile:RuntimeProfile {skript_version:Some("2.15.4".into()),..Default::default()},..Default::default()};
        assert_eq!(config.fuel_per_call,20_000_000);
        let mut host=ParserHost::new(crate::core_library_component(),config).unwrap();
        let source=format!("on join:\n    set {{_source}} to {expression}\n");
        let result=host.parse_document(DocumentParseRequest::new("file:///storage-fuel","file:///storage-fuel/test.sk",1,source.as_str()),DocumentParserConfig::default()).unwrap();
        assert_eq!(result.execution_status,parser_wasm::DocumentExecutionStatus::Complete,"{expression}: {:?}",result.component_failures);
        assert!(result.component_failures.is_empty()&&result.fault_regions.is_empty()&&result.dependency_unavailable.is_empty());
        assert!(result.syntax.diagnostics.is_empty(),"{:?}",result.syntax.diagnostics);
        let start=source.find(expression).unwrap() as u64;let end=start+expression.len() as u64;
        let input=result.ast.nodes.iter().find(|n|n.span.virtual_range.start==start&&n.span.virtual_range.end==end
            &&n.metadata.iter().any(|m|m.key=="skript.semantic.declared-storage-input")).expect("complete RHS declared input");
        let summary=input.summary.as_ref().unwrap();
        assert_eq!(summary.return_type.as_deref(),Some(COMPONENT));assert_eq!(summary.possible_return_types,[COMPONENT]);
        assert_eq!(summary.possible_return_types_state,parser_wasm::bindings::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState::Complete);
        let change=result.ast.nodes.iter().flat_map(|n|&n.metadata).find(|m|m.key=="skript.semantic.variable-change").unwrap();
        let change:Value=serde_json::from_str(&change.value).unwrap();
        assert!(change["storageResult"].is_null(),"authored non-stock registry cannot borrow stock-only Component certification");
        assert_eq!(change["declaredStorage"]["complete"],true);assert_eq!(change["declaredStorage"]["maybeUnset"],true);
        assert!(change["declaredStorage"]["types"].as_array().unwrap().iter().any(|t|t==COMPONENT));
        let document=semantic::Document::from_result(&result,Arc::from(source.as_str()),"file:///storage-fuel/test.sk".parse().unwrap(),Arc::clone(&catalog)).unwrap();
        let hints=document.write_type_hints();assert_eq!(hints.len(),1);
        assert_eq!(hints[0].position,LineIndex::new(&source).position(&source,source.find("{_source}").unwrap()+"{_source}".len()).unwrap());
        assert!(matches!(&hints[0].label,InlayHintLabel::String(label) if label.contains("textcomponent")&&label.ends_with('?')));
    }
}
