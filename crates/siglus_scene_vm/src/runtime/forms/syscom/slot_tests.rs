//! Engine contract tests use synthetic native saves, without game assets.
use super::*;
use crate::formats::gameexe::GameexeConfig;
use crate::runtime::{LocalSaveSnapshot, VmCallMeta};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture {
    ctx: CommandContext,
    dir: PathBuf,
    normal: usize,
    quick: usize,
}

impl Fixture {
    fn new(normal: usize, quick: usize, end: bool) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "siglus-slot-contract-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let config = format!(
            "#SAVE.CNT = {normal}\n#QUICK_SAVE.CNT = {quick}\n#END_SAVE.CNT = {}\n\
             #SAVE_THUMB.USE = 1\n#SAVE_THUMB.TYPE = 1\n",
            usize::from(end)
        );
        fs::write(dir.join("Gameexe.ini"), &config).unwrap();
        let mut ctx = CommandContext::new(dir.clone());
        ctx.tables.gameexe = Some(GameexeConfig::from_text(&config));
        Self {
            ctx,
            dir,
            normal,
            quick,
        }
    }

    fn write(&self, no: usize, title: &str, year: i64) {
        let mut slot = SaveSlotState {
            exist: true,
            year,
            month: 10,
            day: 18,
            title: title.into(),
            message: format!("message {title}"),
            ..Default::default()
        };
        slot.values.insert(2, 777);
        let env = original_save::OriginalLocalSaveEnvelope::from_slot_with_streams(
            &slot,
            vec![1, 2, 3],
            vec![4, 5],
        );
        original_save::write_local_save_file(&self.path(no), &slot, &env).unwrap();
        fs::write(self.thumb(no), title.as_bytes()).unwrap();
    }

    fn path(&self, no: usize) -> PathBuf {
        original_save::save_file_path_for_no(&self.dir, no)
    }

    fn thumb(&self, no: usize) -> PathBuf {
        self.dir.join("savedata").join(format!("{no:04}.png"))
    }

    fn call(&mut self, op: i32, params: &[Value]) -> Option<Value> {
        self.ctx.stack.clear();
        self.ctx.vm_call = Some(VmCallMeta {
            element: vec![super::super::codes::FORM_GLOBAL_SYSCOM as i32, op],
            al_id: 0,
            ret_form: 10,
        });
        assert!(
            dispatch(
                &mut self.ctx,
                super::super::codes::FORM_GLOBAL_SYSCOM,
                params
            )
            .unwrap()
        );
        self.ctx.stack.pop()
    }

    fn int(&mut self, op: i32, params: &[Value]) -> i64 {
        self.call(op, params).unwrap().as_i64().unwrap()
    }

    fn text(&mut self, op: i32, no: usize) -> String {
        self.call(op, &[Value::Int(no as i64)])
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn load_no(&mut self, op: i32, no: i64) -> Option<usize> {
        self.ctx.take_runtime_load_request();
        self.call(
            op,
            &[Value::Int(no), Value::Int(0), Value::Int(1), Value::Int(1)],
        );
        self.ctx
            .take_runtime_load_request()
            .map(|req| match req.kind {
                RuntimeSaveKind::Normal => req.index,
                RuntimeSaveKind::Quick => self.normal + req.index,
                RuntimeSaveKind::End => self.normal + self.quick + req.index,
                RuntimeSaveKind::Inner => panic!("unexpected inner save"),
            })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}

#[test]
fn absolute_metadata_and_load_address_quick_and_end_slots() {
    for (normal, quick, end) in [
        (1, 1, false),
        (10, 3, true),
        (170, 10, true),
        (100, 0, true),
        (3, 5, true),
        (1000, 3, true),
        (4, 0, false),
        (0, 1, false),
        (0, 0, true),
        (0, 0, false),
    ] {
        let mut f = Fixture::new(normal, quick, end);
        assert_eq!(f.int(GET_SAVE_CNT, &[]), normal as i64);
        assert_eq!(f.int(GET_QUICK_SAVE_CNT, &[]), quick as i64);
        if normal > 0 {
            f.write(normal - 1, "ordinary", 2024);
            assert_eq!(f.load_no(LOAD, normal as i64 - 1), Some(normal - 1));
        }
        for index in 0..quick {
            let physical = normal + index;
            f.write(physical, "quick", 2025);
            for (generic, dedicated) in [
                (GET_SAVE_EXIST, GET_QUICK_SAVE_EXIST),
                (GET_SAVE_YEAR, GET_QUICK_SAVE_YEAR),
            ] {
                let expected = if generic == GET_SAVE_EXIST { 1 } else { 2025 };
                assert_eq!(
                    f.int(generic, &[Value::Int(physical as i64)]),
                    expected,
                    "N={normal} Q={quick} physical={physical} op={generic}"
                );
                assert_eq!(f.int(dedicated, &[Value::Int(index as i64)]), expected);
            }
            assert_eq!(f.text(GET_SAVE_TITLE, physical), "quick");
            assert_eq!(f.text(GET_QUICK_SAVE_TITLE, index), "quick");
            assert_eq!(f.load_no(LOAD, physical as i64), Some(physical));
            assert_eq!(f.load_no(QUICK_LOAD, index as i64), Some(physical));
        }
        if end {
            let physical = normal + quick;
            f.write(physical, "end", 2026);
            assert_eq!(f.int(GET_SAVE_EXIST, &[Value::Int(physical as i64)]), 1);
            assert_eq!(f.int(GET_END_SAVE_EXIST, &[]), 1);
            assert_eq!(f.text(GET_SAVE_TITLE, physical), "end");
            assert_eq!(f.load_no(LOAD, physical as i64), Some(physical));
            assert_eq!(f.load_no(END_LOAD, 0), Some(physical));
        }
    }
}

#[test]
fn newest_absolute_slot_includes_quick_end_and_requested_ranges() {
    let mut f = Fixture::new(2, 2, true);
    f.write(0, "ordinary", 2023);
    f.write(2, "quick first", 2025);
    f.write(3, "quick last", 2024);
    f.write(4, "end", 2026);
    assert_eq!(f.int(GET_SAVE_NEW_NO, &[]), 4);
    assert_eq!(f.int(GET_SAVE_NEW_NO, &[Value::Int(2), Value::Int(2)]), 2);
    assert_eq!(f.int(GET_QUICK_SAVE_NEW_NO, &[]), 0);
    assert_eq!(
        f.int(GET_QUICK_SAVE_NEW_NO, &[Value::Int(1), Value::Int(1)]),
        1
    );
    assert_eq!(f.int(GET_SAVE_NEW_NO, &[Value::Int(5), Value::Int(1)]), -1);
}

#[test]
fn aliases_share_metadata_updates_without_rewriting_local_streams() {
    let mut f = Fixture::new(2, 2, true);
    f.write(2, "shared", 2025);
    let packed = fs::read(f.path(2)).unwrap()[original_save::SAVE_HEADER_SIZE..].to_vec();
    assert_eq!(f.int(GET_QUICK_SAVE_EXIST, &[Value::Int(0)]), 1);
    assert_eq!(f.text(GET_SAVE_TITLE, 2), "shared");
    f.call(
        SET_QUICK_SAVE_COMMENT,
        &[Value::Int(0), Value::Str("quick edit".into())],
    );
    assert_eq!(f.text(GET_SAVE_COMMENT, 2), "quick edit");
    f.call(
        SET_SAVE_COMMENT,
        &[Value::Int(2), Value::Str("absolute edit".into())],
    );
    assert_eq!(f.text(GET_QUICK_SAVE_COMMENT, 0), "absolute edit");
    assert!(write_save_flag_values(&mut f.ctx, true, 0, &[11, 22, 33]));
    assert_eq!(
        read_save_flag_values(&mut f.ctx, false, 2, 3),
        Some(vec![11, 22, 33])
    );
    assert!(write_save_flag_values(&mut f.ctx, false, 2, &[44, 55, 66]));
    assert_eq!(
        read_save_flag_values(&mut f.ctx, true, 0, 3),
        Some(vec![44, 55, 66])
    );
    assert_eq!(
        &fs::read(f.path(2)).unwrap()[original_save::SAVE_HEADER_SIZE..],
        packed
    );
}

#[test]
fn copy_swap_and_delete_update_aliases_and_thumbnails_including_negative_cache() {
    let mut f = Fixture::new(2, 2, true);
    f.write(0, "ordinary", 2024);
    assert_eq!(f.int(GET_QUICK_SAVE_EXIST, &[Value::Int(0)]), 0);
    assert_eq!(f.int(GET_END_SAVE_EXIST, &[]), 0);
    assert_eq!(f.int(COPY_SAVE, &[Value::Int(0), Value::Int(2)]), 1);
    assert_eq!(f.int(GET_QUICK_SAVE_EXIST, &[Value::Int(0)]), 1);
    assert_eq!(f.text(GET_QUICK_SAVE_TITLE, 0), "ordinary");
    assert_eq!(fs::read(f.thumb(2)).unwrap(), b"ordinary");
    assert_eq!(f.int(COPY_QUICK_SAVE, &[Value::Int(0), Value::Int(1)]), 1);
    assert_eq!(f.text(GET_SAVE_TITLE, 3), "ordinary");
    assert_eq!(f.int(CHANGE_SAVE, &[Value::Int(0), Value::Int(4)]), 1);
    assert_eq!(f.int(GET_SAVE_EXIST, &[Value::Int(0)]), 0);
    assert_eq!(f.int(GET_END_SAVE_EXIST, &[]), 1);
    assert_eq!(f.text(GET_SAVE_TITLE, 4), "ordinary");
    assert_eq!(fs::read(f.thumb(4)).unwrap(), b"ordinary");
    assert!(!f.thumb(0).exists());
    assert_eq!(f.int(DELETE_QUICK_SAVE, &[Value::Int(0)]), 1);
    assert_eq!(f.int(GET_SAVE_EXIST, &[Value::Int(2)]), 0);
    assert!(!f.path(2).exists());
    assert!(!f.thumb(2).exists());
    assert_eq!(f.int(DELETE_SAVE, &[Value::Int(3)]), 1);
    assert_eq!(f.int(GET_QUICK_SAVE_EXIST, &[Value::Int(1)]), 0);
    assert_eq!(f.int(DELETE_SAVE, &[Value::Int(4)]), 1);
    assert_eq!(f.int(GET_END_SAVE_EXIST, &[]), 0);
}

#[test]
fn disabled_and_invalid_addresses_do_not_alias_existing_slots() {
    let mut f = Fixture::new(2, 1, false);
    for no in 0..=4 {
        f.write(no, "existing", 2025);
    }
    for no in [-1, 3, 4, i64::MAX] {
        assert_eq!(f.int(GET_SAVE_EXIST, &[Value::Int(no)]), 0);
        assert_eq!(f.load_no(LOAD, no), None);
    }
    assert_eq!(f.int(GET_QUICK_SAVE_EXIST, &[Value::Int(1)]), 0);
    assert_eq!(f.load_no(QUICK_LOAD, 1), None);
    assert_eq!(f.int(GET_END_SAVE_EXIST, &[]), 0);
    assert_eq!(f.load_no(END_LOAD, 0), None);
    let original = fs::read(f.path(0)).unwrap();
    for args in [[-1, 0], [0, -1], [0, 3]] {
        assert_eq!(f.int(COPY_SAVE, &args.map(Value::Int)), 0);
        assert_eq!(f.int(CHANGE_SAVE, &args.map(Value::Int)), 0);
    }
    assert_eq!(f.int(DELETE_SAVE, &[Value::Int(-1)]), 0);
    assert_eq!(fs::read(f.path(0)).unwrap(), original);
}

#[test]
fn generic_save_accepts_native_file_numbers_and_respects_end_switch() {
    let mut f = Fixture::new(2, 1, true);
    f.ctx.local_save_snapshot = Some(LocalSaveSnapshot {
        local_stream: vec![1, 2, 3],
        ..Default::default()
    });
    for no in [0, 2, 3] {
        assert_eq!(
            f.int(SAVE, &[Value::Int(no), Value::Int(0), Value::Int(1)]),
            1
        );
        assert_eq!(
            f.ctx.take_runtime_save_request().unwrap().index,
            no as usize
        );
    }
    assert_eq!(f.int(SAVE, &[Value::Int(-1), Value::Int(0)]), 0);
    assert!(f.ctx.take_runtime_save_request().is_none());
    let mut disabled = Fixture::new(2, 0, false);
    disabled.ctx.local_save_snapshot = f.ctx.local_save_snapshot.clone();
    assert_eq!(disabled.int(END_SAVE, &[]), 0);
    assert!(disabled.ctx.take_runtime_save_request().is_none());
}
