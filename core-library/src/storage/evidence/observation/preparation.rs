//! Owner-private staged construction. The completed result is structural only.
use super::*;

pub(in crate::storage) struct Builder { bound:BoundObservation, details_checked:bool, projection_checked:bool }
/// This can only be constructed after the full strict checks. Still not authority.
pub(in crate::storage) struct PreparedProjection(BoundObservation);
impl PreparedProjection {
    pub(in crate::storage) fn into_bound(self)->BoundObservation {self.0}
}
impl Builder {
    pub(in crate::storage) fn new(header:super::super::header::DeferredObservation)->Result<Self,Failure> {
        let source=header.source_identity().clone();
        let mut value=header.header_value()?;
        for name in ["entries","exactBindings","classes","loaders","getters"] {value[name]=serde_json::json!([]);}
        let observation=serde_json::from_value(value).map_err(|_|Failure::Version)?;
        Ok(Self {bound:BoundObservation {source,manifest_fence:ManifestFence::HostCatalogSource,observation},details_checked:false,projection_checked:false})
    }
    pub(in crate::storage) fn append(&mut self,name:&str,rows:Vec<Vec<u8>>)->Result<(),Failure> {
        if self.details_checked||rows.len()>64 {return Err(Failure::Limit);}
        macro_rules! add {($field:ident,$limit:expr)=>{{
            if self.bound.observation.$field.len().saturating_add(rows.len())>$limit {return Err(Failure::Limit);}
            for row in rows {if row.len()>32768{return Err(Failure::Limit);}self.bound.observation.$field.push(serde_json::from_slice(&row).map_err(|_|Failure::Version)?);}
        }}}
        match name {"entries"=>add!(entries,1024),"exactBindings"=>add!(exact_bindings,1024),"classes"=>add!(classes,8192),"loaders"=>add!(loaders,513),"getters"=>add!(getters,4096),_=>return Err(Failure::Version)}
        Ok(())
    }
    pub(in crate::storage) fn check_details(&mut self)->Result<(),Failure> {self.bound.observation.check()?;self.details_checked=true;Ok(())}
    pub(in crate::storage) fn check_projection(&mut self,types:&[Value],events:&[Value])->Result<(),Failure> {
        if !self.details_checked {return Err(Failure::Identity);}
        self.bound.check_projection(types,events)?;self.projection_checked=true;Ok(())
    }
    pub(in crate::storage) fn finish(self)->Result<PreparedProjection,Failure> {
        if !self.details_checked||!self.projection_checked {return Err(Failure::Incomplete);}
        Ok(PreparedProjection(self.bound))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{document, evidence::header};
    fn builder(name:&str)->(Builder,Value,Vec<Value>,Vec<Value>) {
        let (manifest,types,events)=if name=="core" {(
            include_bytes!("data/real-capture/core-manifest.json").as_slice(),include_bytes!("data/real-capture/core-types.json").as_slice(),include_bytes!("data/real-capture/core-event-values.json").as_slice())}else{(
            include_bytes!("data/real-capture/reflect-manifest.json").as_slice(),include_bytes!("data/real-capture/reflect-types.json").as_slice(),include_bytes!("data/real-capture/reflect-event-values.json").as_slice())};
        let m:Value=serde_json::from_slice(manifest).unwrap();let source=SourceIdentity {snapshot_id:m["snapshotId"].as_str().unwrap().into(),source_digest:"a".repeat(64)};
        let doc=document::mock_document(&source,"Manifest.json",||Ok(source.clone()),|offset,max|{let start=offset as usize;Ok(Some(document::Chunk{offset,total_length:manifest.len() as u64,bytes:manifest[start..manifest.len().min(start+max as usize)].to_vec()}))}).unwrap().unwrap();
        let h=header::prepare(&doc,&source).unwrap().1.unwrap().unwrap();
        (Builder::new(h).unwrap(),m,serde_json::from_slice(types).unwrap(),serde_json::from_slice(events).unwrap())
    }
    fn fill(b:&mut Builder,m:&Value){for name in ["entries","exactBindings","classes","loaders","getters"]{for rows in m["registryCapture"][name].as_array().unwrap().chunks(64){b.append(name,rows.iter().map(|r|serde_json::to_vec(r).unwrap()).collect()).unwrap();}}}
    #[test]
    fn actual_core_and_reflect_staged_projection_remains_non_authoritative() {
        for name in ["core","reflect"] {let (mut b,m,types,events)=builder(name);fill(&mut b,&m);b.check_details().unwrap();b.check_projection(&types,&events).unwrap();let projection=b.finish().unwrap().into_bound();
            assert!(projection.is_complete());assert!(projection.storage_gaps(None).contains(&StorageGap::GenerationUnproven));assert!(projection.storage_gaps(None).contains(&StorageGap::ProducerOriginUnproven));}
    }
    #[test]
    fn incomplete_builder_and_conflicting_projection_never_finish() {
        let (b,_,_,_)=builder("core");assert!(matches!(b.finish(),Err(Failure::Incomplete)));
        let (mut b,m,mut types,events)=builder("core");fill(&mut b,&m);b.check_details().unwrap();types[0]["registrationId"]="wrong".into();assert!(b.check_projection(&types,&events).is_err());assert!(b.finish().is_err());
        let (mut b,m,types,events)=builder("core");fill(&mut b,&m);assert!(b.check_projection(&types,&events).is_err());
    }
}
