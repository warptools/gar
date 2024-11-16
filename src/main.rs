use std::fs;

mod add;
mod gittree;
mod repo;

fn main() {
    let r = repo::Repo::new("/tmp");
    r.create_dir_all().expect("waa");
    let hash = add::add(&r, "/tmp/slurpie", add::FaithMode::LinkOriginals).expect("whee");
    println!("{hash:?}")
}
