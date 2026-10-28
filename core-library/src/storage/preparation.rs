//! Source-lifetime structural preparation, separate from storage authorization.
use super::evidence::{SourceIdentity,Failure,observation::preparation::Builder};
use crate::nlaocs::skript_parser_addon::types::{SourcePreparationContext,SourcePreparationStatus,AddonError};
use serde_json::Value;

const ARRAYS:[(&str,usize);7]=[("entries",1024),("exactBindings",1024),("classes",8192),("loaders",513),("getters",4096),("Types.json",1024),("EventValues.json",4096)];
struct Job {generation:u64,next_step:u32,source:SourceIdentity,builder:Builder,phase:usize,offset:u64,total:Option<u64>,types:Vec<Value>,events:Vec<Value>}
impl Job {
    fn accept(&mut self,total:u64,items:Vec<(u64,Vec<u8>)>,next:Option<u64>)->Result<(),Failure> {
        let (name,limit)=ARRAYS[self.phase];
        if total>limit as u64||self.total.is_some_and(|old|old!=total)||items.len()>64 {return Err(Failure::Limit);}
        self.total=Some(total);
        let end=self.offset.checked_add(items.len() as u64).ok_or(Failure::Limit)?;
        if end>total || next!=if end<total {Some(end)} else {None} || end<total&&items.is_empty()
            || items.iter().enumerate().any(|(i,(index,bytes))|*index!=self.offset+i as u64||bytes.len()>32768) {return Err(Failure::Routing);}
        let rows=items.into_iter().map(|(_,b)|b).collect::<Vec<_>>();
        if self.phase<5 {self.builder.append(name,rows)?;}else{
            let target=if self.phase==5 {&mut self.types}else{&mut self.events};
            for bytes in rows {target.push(serde_json::from_slice(&bytes).map_err(|_|Failure::Version)?);}
        }
        if let Some(offset)=next {self.offset=offset;}else{self.phase+=1;self.offset=0;self.total=None;}
        Ok(())
    }
}
#[cfg(target_arch="wasm32")]
static JOB:std::sync::Mutex<Option<Job>>=std::sync::Mutex::new(None);

#[cfg(target_arch="wasm32")]
fn current(context:&SourcePreparationContext)->Option<SourceIdentity> {
    let source=super::document::current_source().ok()?;
    (context.snapshot_id.as_ref()==Some(&source.snapshot_id)&&context.source_digest.as_ref()==Some(&source.source_digest)).then_some(source)
}

#[cfg(target_arch="wasm32")]
pub(crate) fn step(context:SourcePreparationContext)->Result<SourcePreparationStatus,AddonError> {
    use crate::nlaocs::skript_parser_addon::catalog_data;
    let mut slot=JOB.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if context.step==0 {*slot=None;}
    if context.snapshot_id.is_none()&&context.source_digest.is_none() {*slot=None;return Ok(SourcePreparationStatus::Ready("no-catalog-source".into()));}
    let Some(source)=current(&context) else {*slot=None;return Ok(SourcePreparationStatus::Unavailable("source-identity-mismatch".into()));};
    let outcome=(||->Result<SourcePreparationStatus,Failure>{
        if context.generation==0 {return Err(Failure::Subject);}
        if context.step==0 {
            let document=super::document::read_manifest(&source).map_err(|_|Failure::Source)?.ok_or(Failure::Source)?;
            let (_,header)=super::evidence::header::prepare(&document,&super::document::current_source().map_err(|_|Failure::Source)?)?;
            let Some(header)=header? else {return Ok(SourcePreparationStatus::Ready("no-registry-observation".into()));};
            if !header.declared_complete() {return Ok(SourcePreparationStatus::Unavailable("observation-declared-incomplete".into()));}
            *slot=Some(Job{generation:context.generation,next_step:1,source,builder:Builder::new(header)?,phase:0,offset:0,total:None,types:Vec::new(),events:Vec::new()});
            return Ok(SourcePreparationStatus::Pending("manifest-header".into()));
        }
        let job=slot.as_mut().ok_or(Failure::Subject)?;
        if job.generation!=context.generation||job.next_step!=context.step||job.source!=source {return Err(Failure::Subject);}
        job.next_step+=1;
        if job.phase<ARRAYS.len() {
            let phase=job.phase;let name=ARRAYS[phase].0;
            let (document,path,fields)=if phase<5 {("Manifest.json",vec!["registryCapture".into(),name.into()],Vec::new())}
                else if phase==5 {("Types.json",Vec::new(),["originalClass","typeParseOrder","addon","typeRuntime","serializeAs","hasParser","parserClass","registrationId"].map(str::to_owned).to_vec())}
                else {("EventValues.json",Vec::new(),["registrationId","resolutionOrder","eventClass","valueClass","time","registrationOrder","contextDependent","hasCustomInputValidator","hasCustomEventValidator"].map(str::to_owned).to_vec())};
            let page=catalog_data::read_json_records(&source.source_digest,&source.snapshot_id,document,&path,&fields,job.offset,64,32768).map_err(|_|Failure::Source)?.ok_or(Failure::Incomplete)?;
            job.accept(page.total_records,page.items.into_iter().map(|r|(r.index,r.json)).collect(),page.next_offset)?;
            if current(&context).as_ref()!=Some(&source) {return Err(Failure::Source);}
            return Ok(SourcePreparationStatus::Pending(format!("page:{name}")));
        }
        if job.phase==7 {job.builder.check_details()?;job.phase=8;return Ok(SourcePreparationStatus::Pending("detail-identity-check".into()));}
        if job.phase==8 {job.builder.check_projection(&job.types,&job.events)?;job.phase=9;return Ok(SourcePreparationStatus::Pending("projection-check".into()));}
        let job=slot.take().ok_or(Failure::Subject)?;
        let projection=job.builder.finish()?;
        if current(&context).as_ref()!=Some(&source) {return Err(Failure::Source);}
        super::backend::publish_prepared_projection(source,projection)?;
        Ok(SourcePreparationStatus::Ready("registry-projection-ready;generation-and-producer-unproved".into()))
    })();
    match outcome {Ok(value)=>Ok(value),Err(failure)=>{*slot=None;Ok(SourcePreparationStatus::Unavailable(format!("registry-preparation:{failure:?}")))}}
}

#[cfg(not(target_arch="wasm32"))]
pub(crate) fn step(_context:SourcePreparationContext)->Result<SourcePreparationStatus,AddonError> {Ok(SourcePreparationStatus::Ready("native-no-catalog".into()))}

#[cfg(test)]
mod tests {
    use super::*;
    fn job()->Job {
        use super::super::{document,evidence::header};
        let raw=include_bytes!("evidence/observation/data/real-capture/core-manifest.json");
        let value:Value=serde_json::from_slice(raw).unwrap();
        let source=SourceIdentity {snapshot_id:value["snapshotId"].as_str().unwrap().into(),source_digest:"a".repeat(64)};
        let doc=document::mock_document(&source,"Manifest.json",||Ok(source.clone()),|offset,max|{let start=offset as usize;Ok(Some(document::Chunk {offset,total_length:raw.len() as u64,bytes:raw[start..raw.len().min(start+max as usize)].to_vec()}))}).unwrap().unwrap();
        let header=header::prepare(&doc,&source).unwrap().1.unwrap().unwrap();
        Job {generation:1,next_step:1,source,builder:Builder::new(header).unwrap(),phase:1,offset:0,total:None,types:Vec::new(),events:Vec::new()}
    }
    fn row(index:u64)->(u64,Vec<u8>){(index,br#"{"classId":"c0","classInfoInstance":"i0"}"#.to_vec())}
    #[test]
    fn actual_page_admission_rejects_nonadvancement_and_changed_totals() {
        let mut good=job();good.accept(2,vec![row(0)],Some(1)).unwrap();good.accept(2,vec![row(1)],None).unwrap();assert_eq!(good.phase,2);assert!(good.builder.finish().is_err());
        assert!(job().accept(2,vec![],Some(0)).is_err());assert!(job().accept(2,vec![row(1)],Some(1)).is_err());assert!(job().accept(2,vec![row(0)],Some(0)).is_err());
        let mut changed=job();changed.accept(2,vec![row(0)],Some(1)).unwrap();assert!(changed.accept(3,vec![row(1)],Some(2)).is_err());
        assert!(job().accept(65,(0..65).map(row).collect(),None).is_err());
    }
}
