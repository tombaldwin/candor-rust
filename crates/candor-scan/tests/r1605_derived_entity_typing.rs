//! SOUNDNESS R1605 — a sea_orm entity declared in the CONSUMER by `#[derive(DeriveEntityModel)]`. The derive generates
//! `Entity` (and its `impl EntityTrait`), which the scan cannot see, so `post::Entity::find().all(db)` named a type no item
//! declares, the `Select` that `find()` returns stayed untyped, and the SELECT reached no rule: the caller ABSENT from
//! `functions[]`, no `invisible`, `deny Db` and `deny Unknown` both exit 0 (EXECUTED in the lane on file-backed sqlite with
//! sea_orm's statement callback, `~/candor-handoff/rust-seaent/`).
//!
//! The fix splices the derive's DECLARED skeleton (`struct Entity;` + an empty `impl sea_orm::entity::EntityTrait for
//! Entity`) and types each receiver-less member the local type does not declare from the TRAIT'S DECLARATION in the
//! lock-pinned source. Without a lock-pinned source nothing is typed and the caller discloses `Unknown` instead.
//!
//! Hermetic: the dependency is a `path` dependency carrying the classified crate's NAME (`sea-orm`) and a minimal copy of
//! the trait and query types, so the classifier's own `sea_orm` rules are what charges. `CANDOR_R1605_OFF=1` restores the
//! published behaviour; every charged case must then read uncharged — the proof the fixtures reach the new code.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1605-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

/// sea-orm's shape where it matters: `EntityTrait` declared once in `entity::base_entity`, its provided `find` returning
/// `Select<Self>`, `insert` a generic `Insert<A>`, a member returning `Self` (never typed), the query types in `query::*`
/// with their terminals, and the sea_query builder `Condition` re-exported at the root. `extra` is appended to `lib.rs`.
fn dep(root: &Path, extra: &str) {
    let d = root.join("sea-orm");
    write(&d.join("Cargo.toml"), "[package]\nname = \"sea-orm\"\nversion = \"2.0.4\"\nedition = \"2021\"\n");
    write(
        &d.join("src/lib.rs"),
        &format!("pub mod entity;\npub mod query;\npub use entity::*;\npub use query::*;\npub struct DatabaseConnection;\n{extra}"),
    );
    write(&d.join("src/entity/mod.rs"), "pub mod base_entity;\npub mod prelude;\npub use base_entity::*;\n");
    write(
        &d.join("src/entity/base_entity.rs"),
        "use crate::{Select, Insert, DeleteMany};\n\
         pub trait EntityTrait: Sized {\n\
         \x20   fn find() -> Select<Self> { Select::new() }\n\
         \x20   fn find_by_id<T>(_v: T) -> Select<Self> { Select::new() }\n\
         \x20   fn insert<A>(_m: A) -> Insert<A> { Insert::new() }\n\
         \x20   fn delete_many() -> DeleteMany<Self> { DeleteMany::new() }\n\
         \x20   fn me() -> Self;\n\
         }\n",
    );
    write(&d.join("src/entity/prelude.rs"), "pub use crate::{DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, Select};\n");
    write(&d.join("src/query/mod.rs"), "pub mod select;\npub mod helper;\npub use select::*;\npub use helper::*;\n");
    write(
        &d.join("src/query/select.rs"),
        "pub struct Select<E>(Option<E>);\n\
         impl<E> Select<E> { pub fn new() -> Self { Select(None) } \
         pub async fn all(self, _db: &crate::DatabaseConnection) {} pub async fn one(self, _db: &crate::DatabaseConnection) {} }\n\
         pub trait QueryFilter: Sized { fn filter(self, _c: u8) -> Self { self } }\n\
         macro_rules! impl_filter { ($t:ident) => { impl<E> $t for Select<E> {} } }\n\
         impl_filter!(QueryFilter);\n\
         pub struct Paginator;\n\
         impl Paginator { pub async fn num_items(&self) -> u64 { 0 } }\n\
         pub trait PaginatorTrait: Sized { fn paginate(self, _db: &crate::DatabaseConnection, _n: u64) -> Paginator { Paginator } }\n\
         impl<E> PaginatorTrait for Select<E> {}\n\
         pub struct Insert<A>(Option<A>);\n\
         impl<A> Insert<A> { pub fn new() -> Self { Insert(None) } pub async fn exec(self, _db: &crate::DatabaseConnection) {} }\n\
         pub struct DeleteMany<E>(Option<E>);\n\
         impl<E> DeleteMany<E> { pub fn new() -> Self { DeleteMany(None) } pub async fn exec(self, _db: &crate::DatabaseConnection) {} }\n",
    );
    write(&d.join("src/query/helper.rs"), "pub struct Condition;\nimpl Condition { pub fn all() -> Self { Condition } }\n");
}

/// The consumer: an entity in a sea-orm-cli layout (`entities/post.rs` + `prelude.rs`'s `pub use .. as Post`), the
/// query idiom in several spellings (`t_*`), pure builders over the same values (`c_*`), and the root-spelled
/// sea_query builder (`f_cond_all`). `body` replaces the default `lib.rs` when given.
fn consumer(root: &Path, name: &str, lock: bool, body: Option<&str>) -> PathBuf {
    let d = root.join(name);
    write(
        &d.join("Cargo.toml"),
        &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nsea-orm = {{ path = \"../sea-orm\" }}\n"),
    );
    if lock {
        write(
            &d.join("Cargo.lock"),
            &format!("version = 4\n\n[[package]]\nname = \"sea-orm\"\nversion = \"2.0.4\"\n\n[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
        );
    }
    write(&d.join("src/entities/mod.rs"), "pub mod post;\npub mod prelude;\n");
    write(&d.join("src/entities/prelude.rs"), "pub use super::post::Entity as Post;\n");
    write(
        &d.join("src/entities/post.rs"),
        "use sea_orm::entity::prelude::*;\n\
         #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]\n\
         #[sea_orm(table_name = \"post\")]\n\
         pub struct Model { #[sea_orm(primary_key)] pub id: i32 }\n\
         pub async fn t_in_mod(db: &DatabaseConnection) { Entity::find().all(db).await }\n",
    );
    let default = "mod entities;\n\
         use entities::post;\n\
         use sea_orm::{Condition, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter};\n\
         pub struct Handle;\n\
         impl Handle { pub fn find(&self) -> Handle { Handle } }\n\
         pub async fn t_generic_leaf<E: EntityTrait>(db: &DatabaseConnection) { let mut s = E::find().filter(1); s = s.filter(2); s.all(db).await }\n\
         pub async fn t_paginate_let(db: &DatabaseConnection) { let p = post::Entity::find().filter(1).paginate(db, 10); p.num_items().await; }\n\
         pub trait Aud: EntityTrait {}\n\
         pub async fn t_generic<E: EntityTrait>(db: &DatabaseConnection) { E::find().all(db).await }\n\
         pub async fn t_generic_let<E: EntityTrait>(db: &DatabaseConnection) { let q = E::find().filter(1).filter(2); q.all(db).await }\n\
         pub async fn t_generic_super<E: Aud>(db: &DatabaseConnection) { E::find_by_id(1).one(db).await }\n\
         pub async fn t_entity_let(db: &DatabaseConnection) { let q = post::Entity::find().filter(1); q.all(db).await }\n\
         pub fn c_generic_default<E: EntityTrait + Default>() -> E { E::default() }\n\
         pub async fn t_find_all(db: &DatabaseConnection) { post::Entity::find().all(db).await }\n\
         pub async fn t_by_id_one(db: &DatabaseConnection) { post::Entity::find_by_id(1).one(db).await }\n\
         pub async fn t_chain(db: &DatabaseConnection) { post::Entity::find().filter(1).filter(2).all(db).await }\n\
         pub async fn t_bound(db: &DatabaseConnection) { let q = post::Entity::find().filter(1); q.all(db).await }\n\
         pub async fn t_cond_all(db: &DatabaseConnection) { post::Entity::find().filter(Condition::all()).all(db).await }\n\
         pub async fn t_insert(db: &DatabaseConnection) { post::Entity::insert(3u8).exec(db).await }\n\
         pub async fn t_delete_many(db: &DatabaseConnection) { post::Entity::delete_many().exec(db).await }\n\
         pub async fn t_alias(db: &DatabaseConnection) { entities::prelude::Post::find().all(db).await }\n\
         pub fn c_filter_only() -> sea_orm::Select<post::Entity> { post::Entity::find().filter(1) }\n\
         pub fn f_cond_all() -> Condition { Condition::all() }\n";
    write(&d.join("src/lib.rs"), body.unwrap_or(default));
    d
}

/// Every query terminal of the default consumer, by the name the report gives it. `t_cond_all` is left out: the unflipped
/// R1605 tripwire charges its `Condition::all()` builder `Db` in every arm, so it cannot tell the routes apart.
const CHARGED: &[&str] = &[
    "t_find_all", "t_by_id_one", "t_chain", "t_bound", "t_insert", "t_delete_many", "t_alias",
    "entities::post::t_in_mod", "t_generic", "t_generic_let", "t_generic_super", "t_entity_let", "t_generic_leaf",
    "t_paginate_let",
];

/// `fn -> (inferred, unknownWhy)`; the scan is never chained.
fn scan(dir: &Path, off: bool) -> HashMap<String, (Vec<String>, Vec<String>)> {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json").env_remove("CANDOR_DEPS").env_remove("CANDOR_R1605_OFF")
        .env_remove("CANDOR_R1578_OFF").env_remove("CANDOR_R1580").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY");
    if off {
        cmd.env("CANDOR_R1605_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    assert!(out.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    let strs = |x: &serde_json::Value| -> Vec<String> {
        x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    v["functions"].as_array().unwrap().iter()
        .map(|f| (f["fn"].as_str().unwrap().to_string(), (strs(&f["inferred"]), strs(&f["unknownWhy"]))))
        .collect()
}

fn has(rows: &HashMap<String, (Vec<String>, Vec<String>)>, f: &str, e: &str) -> bool {
    rows.get(f).is_some_and(|r| r.0.iter().any(|x| x == e))
}

const WHY: &str = "dispatch:untyped cross-package receiver";

#[test]
fn a_derived_entity_query_terminal_is_charged_through_the_trait_declaration_with_a_lock() {
    let root = tmpdir("lock");
    dep(&root, "");
    let app = consumer(&root, "app_lock", true, None);
    let on = scan(&app, false);
    for f in CHARGED {
        assert!(has(&on, f, "Db"), "`{f}` must be charged Db through EntityTrait's declared return; row {:?}", on.get(*f));
    }
    // a pure builder over the same value stays uncharged. (`f_cond_all`, the root-spelled sea_query builder, is still
    // charged `Db` by the R1605 tripwire in `candor-classify`; flipping it is a separate change — see the row.)
    assert!(!has(&on, "c_filter_only", "Db"), "pure builder charged: {:?}", on.get("c_filter_only"));
    assert!(!has(&on, "c_generic_default", "Unknown") && !has(&on, "c_generic_default", "Db"),
        "`E::default()` is not the trait's value: {:?}", on.get("c_generic_default"));
    // typed, so no hedge was needed
    for f in CHARGED {
        assert!(!has(&on, f, "Unknown"), "`{f}` was typed and must not also be hedged: {:?}", on.get(*f));
    }
    // OFF: the published behaviour — every terminal reads uncharged again, so the fixture reaches the new code
    // (`t_in_mod` is left out here: with the splice off, the bare `Entity` is read as the dependency prelude glob's, the
    // crate's leaf record for `find` types it `Handle`, and `.all` is charged through the glob-retyped
    // `sea_orm::entity::prelude::Handle::all` — a guess that happens to land, the published route, not R1605's.)
    let off = scan(&app, true);
    for f in CHARGED.iter().filter(|f| **f != "entities::post::t_in_mod") {
        assert!(!has(&off, f, "Db"), "with CANDOR_R1605_OFF `{f}` still charged — the fixture does not reach R1605");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn without_a_lock_pinned_source_the_untyped_terminal_discloses_and_charges_nothing() {
    let root = tmpdir("nolock");
    dep(&root, "");
    let app = consumer(&root, "app_nolock", false, None);
    let rows = scan(&app, false);
    for f in CHARGED {
        assert!(!has(&rows, f, "Db"), "`{f}` was typed without a lock pinning the source: {:?}", rows.get(*f));
        assert!(
            has(&rows, f, "Unknown") && rows[*f].1.iter().any(|w| w == WHY),
            "`{f}` must disclose `Unknown` ({WHY}) when the member's return cannot be read; row {:?}",
            rows.get(*f)
        );
    }
    assert!(!has(&rows, "c_generic_default", "Unknown"), "`E::default()` was hedged: {:?}", rows.get("c_generic_default"));
    let off = scan(&app, true);
    for f in CHARGED {
        assert!(!has(&off, f, "Unknown"), "with CANDOR_R1605_OFF `{f}` still discloses — the floor is not R1605's");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_macro_that_may_declare_the_trait_refuses_the_typing_and_the_terminal_discloses() {
    // A local `macro_rules!` whose body writes `trait EntityTrait` may declare (part of) it: the declaration read is not
    // the whole truth, so nothing is typed — and the caller discloses rather than going silent.
    let root = tmpdir("macro");
    dep(&root, "macro_rules! twin { () => { pub trait EntityTrait {} } }\n");
    let app = consumer(&root, "app_macro", true, None);
    let rows = scan(&app, false);
    assert!(!has(&rows, "t_find_all", "Db"), "typed through a refused declaration: {:?}", rows.get("t_find_all"));
    assert!(has(&rows, "t_find_all", "Unknown"), "refused typing must disclose: {:?}", rows.get("t_find_all"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_local_inherent_member_wins_and_a_written_impl_is_typed_like_the_derived_one() {
    // `impl Entity { fn find() -> Local }` written by the crate shadows the trait's provided `find` in Rust's resolution, so
    // its declared return is the one used (no Db through `Local::all`); a HAND-WRITTEN `impl EntityTrait for Hand {}` is
    // the same fact the derive's skeleton states, and is typed the same way.
    let root = tmpdir("local");
    dep(&root, "");
    let body = "use sea_orm::{DatabaseConnection, EntityTrait};\n\
                pub struct Local;\n\
                impl Local { pub async fn all(&self, _db: &DatabaseConnection) {} }\n\
                pub mod own {\n\
                \x20   use sea_orm::entity::prelude::*;\n\
                \x20   #[derive(DeriveEntityModel)]\n\
                \x20   #[sea_orm(table_name = \"own\")]\n\
                \x20   pub struct Model { pub id: i32 }\n\
                \x20   impl Entity { pub fn find() -> crate::Local { crate::Local } }\n\
                }\n\
                pub struct Hand;\n\
                impl sea_orm::entity::EntityTrait for Hand { fn me() -> Self { Hand } }\n\
                pub async fn c_inherent_wins(db: &DatabaseConnection) { own::Entity::find().all(db).await }\n\
                pub async fn t_hand(db: &DatabaseConnection) { Hand::find().all(db).await }\n";
    let app = consumer(&root, "app_local", true, Some(body));
    let rows = scan(&app, false);
    assert!(!has(&rows, "c_inherent_wins", "Db"), "the local inherent `find` was overridden: {:?}", rows.get("c_inherent_wins"));
    assert!(has(&rows, "t_hand", "Db"), "a written EntityTrait impl must type like the derived one: {:?}", rows.get("t_hand"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_builder_step_is_followed_only_when_every_declaring_trait_returns_self() {
    // A second trait in the source, blanket-implemented for every type, declares `filter` returning a different type: which
    // one the consumer's call reaches is not decided here, so the let-bound chain stays untyped (and, with the lock, discloses nothing new — the published
    // behaviour). The direct chain, typed by the collector's own builder walk, still charges.
    let root = tmpdir("twin");
    dep(&root, "pub struct Other;\npub trait Narrow { fn filter(self, _c: u8) -> Other where Self: Sized { Other } }\nimpl<T> Narrow for T {}\n");
    let app = consumer(&root, "app_twin", true, None);
    let rows = scan(&app, false);
    assert!(!has(&rows, "t_generic_let", "Db"), "a step two traits declare differently was followed: {:?}", rows.get("t_generic_let"));
    assert!(has(&rows, "t_generic", "Db"), "the direct terminal must still charge: {:?}", rows.get("t_generic"));
    let _ = std::fs::remove_dir_all(&root);
}

/// SOUNDNESS R1607 — a dot-call on a local value whose type implements a dependency trait without declaring the method:
/// `am.insert(db)` on the derive's `ActiveModel`, `model.delete(db)` on `Model`, and the same on a HAND-WRITTEN
/// `impl sea_orm::ActiveModelTrait for Hand {}`. The call is put to the classifier as `sea_orm::ActiveModelTrait::insert`
/// (`ModelTrait::delete`) and charged only when the lock-pinned trait declaration names the member; without a lock the
/// caller discloses `Unknown`. The pure members (`is_changed`, `get`) never charge — the derive's overrides are spliced as
/// empty local members, and the classifier names only the query members.
fn r1607_dep(root: &Path) {
    dep(
        root,
        "pub trait ActiveModelTrait: Sized { fn get(&self) {} fn is_changed(&self) -> bool { false } \
         async fn insert(self, _db: &DatabaseConnection) {} async fn save(self, _db: &DatabaseConnection) {} }\n\
         pub trait ModelTrait: Sized { async fn delete(self, _db: &DatabaseConnection) {} }\n",
    );
}

const R1607_BODY: &str = "mod entities;\n\
     use entities::post;\n\
     use sea_orm::{ActiveModelTrait, DatabaseConnection, ModelTrait};\n\
     pub struct Hand;\n\
     impl sea_orm::ActiveModelTrait for Hand {}\n\
     pub async fn u_am_insert(db: &DatabaseConnection) { post::ActiveModel.insert(db).await }\n\
     pub async fn u_am_save(db: &DatabaseConnection) { let am = post::ActiveModel; am.save(db).await }\n\
     pub async fn u_model_delete(m: post::Model, db: &DatabaseConnection) { m.delete(db).await }\n\
     pub async fn u_hand_insert(db: &DatabaseConnection) { Hand.insert(db).await }\n\
     pub fn c_am_changed() -> bool { post::ActiveModel.is_changed() }\n\
     pub fn c_hand_changed() -> bool { Hand.is_changed() }\n\
     pub fn c_am_get() { post::ActiveModel.get() }\n";

const R1607_CHARGED: &[&str] = &["u_am_insert", "u_am_save", "u_model_delete", "u_hand_insert"];

#[test]
fn a_dependency_trait_member_called_on_a_local_value_is_classified_under_the_trait() {
    let root = tmpdir("r1607");
    r1607_dep(&root);
    let lock = scan(&consumer(&root, "app_r1607", true, Some(R1607_BODY)), false);
    for f in R1607_CHARGED {
        assert!(has(&lock, f, "Db"), "`{f}` must be charged Db under the trait's spelling; row {:?}", lock.get(*f));
    }
    for f in ["c_am_changed", "c_hand_changed", "c_am_get"] {
        assert!(!has(&lock, f, "Db") && !has(&lock, f, "Unknown"), "`{f}` is a pure member: {:?}", lock.get(f));
    }
    let nolock = scan(&consumer(&root, "app_r1607_nl", false, Some(R1607_BODY)), false);
    for f in R1607_CHARGED {
        assert!(!has(&nolock, f, "Db"), "`{f}` charged without a lock-pinned source: {:?}", nolock.get(*f));
        assert!(has(&nolock, f, "Unknown"), "`{f}` must disclose without a lock-pinned source: {:?}", nolock.get(*f));
    }
    let app = consumer(&root, "app_r1607_off", true, Some(R1607_BODY));
    let mut cmd = Command::new(bin());
    cmd.arg(app.to_string_lossy().as_ref()).arg("--json").env("CANDOR_R1607_OFF", "1").env_remove("CANDOR_DEPS");
    let out = cmd.output().expect("run candor-scan");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    for f in R1607_CHARGED {
        let row = v["functions"].as_array().unwrap().iter().find(|r| r["fn"] == *f);
        let db = row.is_some_and(|r| r["inferred"].as_array().is_some_and(|a| a.iter().any(|e| e == "Db")));
        assert!(!db, "with CANDOR_R1607_OFF `{f}` still charged — the fixture does not reach R1607");
    }
    let _ = std::fs::remove_dir_all(&root);
}
