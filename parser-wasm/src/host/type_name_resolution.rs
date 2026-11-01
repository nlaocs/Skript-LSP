//! Bounded named-type lookup: current inflection, exact code name, then user regex.
use super::*;

pub(super) fn resolve(catalog:&Catalog,matchers:&[TypeUserInputMatcher],input:&str)
    ->Result<Option<(WitExpressionTypeOption,bool)>,wit_catalog_data::CatalogError>
{
    if input.len()>1024 {return Err(invalid_catalog_input("type name exceeds 1024 bytes"));}
    if input.is_empty(){return Ok(None);}
    let (singular,plural)=catalog.plural_rules().to_singular(input);
    let mut exact=catalog.types().filter(|ty|ty.code_name.as_str()==singular);
    if let Some(ty)=exact.next() {
        if exact.next().is_some(){return Err(invalid_catalog_input("type code name is ambiguous"));}
        return Ok(Some((expression_type_option(Some(catalog),ty),plural)));
    }
    for matcher in matchers {
        for pattern in &matcher.patterns {
            if pattern.is_match(&singular).map_err(|error|invalid_catalog_input(format!("type user-input regex failed: {error}")))? {
                return Ok(Some((matcher.option.clone(),plural)));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot()->ssg::Snapshot {
        ssg::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")).unwrap()
    }
    #[test]
    fn named_types_use_code_name_before_regex_and_preserve_recorded_plural_rules() {
        let source=snapshot();let catalog=source.catalog();let matchers=build_type_user_input_matchers(Some(catalog));
        for (word,class,plural) in [
            ("number","java.lang.Number",false),("numbers","java.lang.Number",true),
            ("string","java.lang.String",false),("strings","java.lang.String",true),
            ("text","java.lang.String",false),("texts","java.lang.String",true),
            ("integer","java.lang.Integer",false),("integers","java.lang.Integer",true),
            ("double","java.lang.Double",false),("doubles","java.lang.Double",true),
        ] {
            let (ty,multiple)=resolve(catalog,&matchers,word).unwrap().unwrap();assert_eq!(ty.class_name,class,"{word}");assert_eq!(multiple,plural,"{word}");
        }
        assert!(resolve(catalog,&matchers,"a-missing-current-type").unwrap().is_none());
        assert!(resolve(catalog,&matchers,"").unwrap().is_none());
        assert!(resolve(catalog,&matchers,&"x".repeat(1025)).is_err());
        // The old raw-regex resolver still has its distinct contract. The new
        // resolver must not select Long merely because that regex accepts integer.
        let raw=matchers.iter().find(|m|m.patterns.iter().any(|p|p.is_match("integer").unwrap_or(false))).unwrap();
        assert_ne!(raw.option.class_name,"java.lang.Integer");
    }
    #[test]
    fn named_types_use_the_captured_override_instead_of_an_english_suffix_guess() {
        let source=snapshot();let current=source.catalog();
        let mut ty=current.types().find(|ty|ty.original_class.as_str()=="java.lang.String").unwrap().clone();
        ty.code_name=syntaxes::TypeCodeName("dummyfixtureperson".into());
        let custom=Catalog::new(syntaxes::CatalogParts {
            syntaxes:vec![syntaxes::Syntax::Type(ty)],converters:vec![],comparators:vec![],event_values:vec![],
            properties:vec![],operators:vec![],operations:BTreeMap::new(),differences:vec![],classes:vec![],
            aliases:syntaxes::AliasRegistry {aliases:BTreeMap::new(),targets:vec![]},
            plural_rules:current.plural_rules().clone(),language:BTreeMap::new(),
        });
        for (word,multiple) in [("dummyfixtureperson",false),("dummyfixturepeople",true)] {
            let (ty,plural)=resolve(&custom,&[],word).unwrap().unwrap();
            assert_eq!(ty.class_name,"java.lang.String");assert_eq!(plural,multiple);
        }
    }

    #[test]
    fn named_type_api_requires_catalog_v7_and_rejects_a_v6_host() {
        let required=[crate::CapabilityRequirement::required(CAPABILITY_CATALOG_DATA,7)];
        let base=host_capabilities();
        assert!(base.iter().all(|cap|cap.id!=CAPABILITY_CATALOG_DATA),"a host without source data cannot claim this API");
        let current=configured_host_capabilities(true,true);
        assert!(current.iter().any(|cap|cap.id==CAPABILITY_CATALOG_DATA && cap.version==7));
        let mut prior=current.clone();prior.iter_mut().find(|cap|cap.id==CAPABILITY_CATALOG_DATA).unwrap().version=6;
        let abi=crate::AbiVersion::new(31,0);
        assert!(crate::validate_compatibility(abi,abi,&required,&prior).is_err());
        crate::validate_compatibility(abi,abi,&required,&current).unwrap();
    }
}
