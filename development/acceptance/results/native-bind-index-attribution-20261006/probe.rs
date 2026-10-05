use std::{collections::HashMap, hint::black_box, time::Instant};
#[derive(Clone,Eq,PartialEq,Hash)] struct RootId(String);
#[derive(Clone)] struct Identity(Vec<u8>);
struct Record { identity: Identity, attr: u64 }
struct State { paths: HashMap<(RootId,String),u64>, inodes: HashMap<u64,Record>, identities: HashMap<(RootId,Vec<u8>),u64> }
impl State {
fn rebuild_identity_index(&mut self) {
        self.identities.clear();
        for ((root_id, _path), inode) in &self.paths {
            if let Some(record) = self.inodes.get(inode) {
                self.identities
                    .insert((root_id.clone(), record.identity.0.clone()), *inode);
            }
        }
    }
}
fn main() {
 for n in [100usize,1000,10000] {
  let root=RootId("root-6167656e7431".to_string());
  let mut s=State {paths:HashMap::new(),inodes:HashMap::new(),identities:HashMap::new()};
  for i in 0..n {s.paths.insert((root.clone(),format!("f{i:05}")),i as u64);s.inodes.insert(i as u64,Record {identity:Identity((i as u128).to_le_bytes().to_vec()),attr:0});}
  s.rebuild_identity_index();assert_eq!(s.identities.len(),n);
  let repeats=200;
  let start=Instant::now();
  for i in 0..repeats {s.inodes.get_mut(&0).unwrap().attr=i;s.rebuild_identity_index();black_box(&s);}
  let rebuild=start.elapsed().as_nanos();
  let start=Instant::now();
  for i in 0..repeats {s.inodes.get_mut(&0).unwrap().attr=i;black_box(&s);}
  let attr_only=start.elapsed().as_nanos();
  assert_eq!(s.identities.len(),n);
  println!("{{\"paths\":{},\"repeats\":{},\"rebuild_total_ns\":{},\"attribute_only_total_ns\":{},\"index_count_ok\":true}}",n,repeats,rebuild,attr_only);
 }
}
