pub type Hash = [u8; 32];
pub const DEPTH: usize = 32;

pub mod db;
pub mod path;
pub mod prevhash;
pub mod proof;
pub mod trees;
pub mod vote_generated;

pub struct Election {
    pub name: String,
    pub start_height: u32,
    pub end_height: u32,
}
