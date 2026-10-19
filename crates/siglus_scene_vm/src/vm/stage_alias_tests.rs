use super::*;
use crate::runtime::forms::codes;
use std::path::PathBuf;

const STAGES: [(i32, i32, i32); 3] = [
    (codes::ELM_GLOBAL_BACK, 0, 2),
    (codes::ELM_GLOBAL_FRONT, 1, 5),
    (codes::ELM_GLOBAL_NEXT, 2, 7),
];

fn fixture(program: &[u8]) -> SceneVm<'static> {
    let mut words = [0i32; 33];
    for i in [
        0usize, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 29, 31,
    ] {
        words[i] = 132;
    }
    words[2] = program.len() as i32;
    let mut chunk = Vec::new();
    for word in words {
        chunk.extend_from_slice(&word.to_le_bytes());
    }
    chunk.extend_from_slice(program);
    let stream = SceneStream::new(Box::leak(chunk.into_boxed_slice())).unwrap();
    SceneVm::new(
        stream,
        CommandContext::new(PathBuf::from("/nonexistent/siglus-stage-alias-tests")),
    )
}

fn canonical(vm: &SceneVm<'_>, stage: i32, child: i32, op: i32) -> Vec<i32> {
    vec![
        vm.ctx.ids.form_global_stage as i32,
        vm.ctx.ids.elm_array,
        stage,
        child,
        op,
    ]
}

fn command(vm: &mut SceneVm<'_>, chain: Vec<i32>, args: &[i64]) {
    vm.exec_command(
        chain,
        0,
        vm.cfg.fm_void,
        &mut args.iter().copied().map(Value::Int).collect(),
    )
    .unwrap();
}

fn query(vm: &mut SceneVm<'_>, chain: Vec<i32>) -> i32 {
    vm.exec_command(chain, 0, vm.cfg.fm_int, &mut vec![])
        .unwrap();
    vm.pop_int().unwrap()
}

fn seed(vm: &mut SceneVm<'_>) {
    for (_, stage, count) in STAGES {
        let chain = canonical(
            vm,
            stage,
            vm.ctx.ids.stage_elm_object,
            codes::OBJECTLIST_RESIZE,
        );
        command(vm, chain, &[count as i64]);
    }
}

#[test]
fn bytecode_front_object_get_size_returns_the_real_list_length() {
    let mut program = vec![constants::cd::ELM_POINT];
    for element in [
        codes::ELM_GLOBAL_FRONT,
        codes::STAGE_ELM_OBJECT,
        codes::OBJECTLIST_GET_SIZE,
    ] {
        program.push(constants::cd::PUSH);
        program.extend_from_slice(&constants::fm::INT.to_le_bytes());
        program.extend_from_slice(&element.to_le_bytes());
    }
    program.push(constants::cd::COMMAND);
    for value in [0i32, 0, 0, constants::fm::INT] {
        program.extend_from_slice(&value.to_le_bytes());
    }
    program.push(constants::cd::EOF);
    let mut vm = fixture(&program);
    seed(&mut vm);
    vm.run_script_proc()
        .expect("front.object.get_size bytecode");
    assert_eq!(vm.pop_int().unwrap(), 5);
}

#[test]
fn aliases_query_distinct_back_front_and_next_object_lists() {
    let mut vm = fixture(&[]);
    seed(&mut vm);
    for (alias, stage, count) in STAGES {
        assert_eq!(
            query(
                &mut vm,
                vec![alias, codes::STAGE_ELM_OBJECT, codes::OBJECTLIST_GET_SIZE]
            ),
            count
        );
        let chain = canonical(
            &vm,
            stage,
            codes::STAGE_ELM_OBJECT,
            codes::OBJECTLIST_GET_SIZE,
        );
        assert_eq!(query(&mut vm, chain), count);
    }
}

#[test]
fn aliases_resize_only_the_selected_object_list() {
    for (alias, selected, _) in STAGES {
        let mut vm = fixture(&[]);
        seed(&mut vm);
        for size in [0, 11, 3] {
            command(
                &mut vm,
                vec![alias, codes::STAGE_ELM_OBJECT, codes::OBJECTLIST_RESIZE],
                &[size],
            );
            assert_eq!(
                query(
                    &mut vm,
                    vec![alias, codes::STAGE_ELM_OBJECT, codes::OBJECTLIST_GET_SIZE]
                ),
                size as i32
            );
            for (_, stage, original) in STAGES {
                let chain = canonical(
                    &vm,
                    stage,
                    codes::STAGE_ELM_OBJECT,
                    codes::OBJECTLIST_GET_SIZE,
                );
                assert_eq!(
                    query(&mut vm, chain),
                    if stage == selected {
                        size as i32
                    } else {
                        original
                    }
                );
            }
        }
    }
}

#[test]
fn aliases_dispatch_stage_create_commands() {
    let mut vm = fixture(&[]);
    seed(&mut vm);
    for (alias, stage, _) in STAGES {
        command(&mut vm, vec![alias, 0], &[4]);
        command(&mut vm, vec![alias, 1], &[2]);
        let chain = canonical(
            &vm,
            stage,
            codes::STAGE_ELM_OBJECT,
            codes::OBJECTLIST_GET_SIZE,
        );
        assert_eq!(query(&mut vm, chain), 4);
        let state = &vm.ctx.globals.stage_forms[&vm.ctx.ids.form_global_stage];
        assert_eq!(state.mwnd_lists[&(stage as i64)].len(), 2);
    }
}

#[test]
fn aliases_dispatch_effect_list_commands() {
    let mut vm = fixture(&[]);
    for (alias, stage, count) in STAGES {
        command(
            &mut vm,
            vec![alias, codes::STAGE_ELM_EFFECT, constants::EFFECTLIST_RESIZE],
            &[count as i64],
        );
        assert_eq!(
            query(
                &mut vm,
                vec![
                    alias,
                    codes::STAGE_ELM_EFFECT,
                    constants::EFFECTLIST_GET_SIZE
                ]
            ),
            count
        );
        let chain = canonical(
            &vm,
            stage,
            codes::STAGE_ELM_EFFECT,
            constants::EFFECTLIST_GET_SIZE,
        );
        assert_eq!(query(&mut vm, chain), count);
    }
}

#[test]
fn indexed_and_nested_alias_objects_keep_their_indices() {
    let mut vm = fixture(&[]);
    seed(&mut vm);
    for (object, width) in [
        (
            vec![codes::ELM_GLOBAL_FRONT, codes::STAGE_ELM_OBJECT, -1, 0],
            50,
        ),
        (
            vec![
                codes::ELM_GLOBAL_FRONT,
                codes::STAGE_ELM_OBJECT,
                -1,
                0,
                codes::ELM_OBJECT_CHILD,
                -1,
                1,
            ],
            17,
        ),
    ] {
        let mut create = object.clone();
        create.push(codes::ELM_OBJECT_CREATE_RECT);
        vm.exec_command(
            create,
            2,
            vm.cfg.fm_void,
            &mut [0, 0, width, 40, 255, 255, 255, 255, 1, 0, 0]
                .into_iter()
                .map(Value::Int)
                .collect(),
        )
        .unwrap();
        let mut get = object;
        get.push(codes::ELM_OBJECT_GET_SIZE_X);
        assert_eq!(query(&mut vm, get), width as i32);
    }
    for (_, stage, count) in STAGES {
        assert_eq!(
            query(
                &mut vm,
                vec![
                    codes::ELM_GLOBAL_STAGE,
                    codes::STAGE_ELM_OBJECT,
                    -1,
                    stage,
                    codes::OBJECTLIST_GET_SIZE
                ]
            ),
            count
        );
    }
}

#[test]
fn excall_private_stage_remains_separate_from_global_aliases() {
    let mut vm = fixture(&[]);
    seed(&mut vm);
    command(
        &mut vm,
        vec![codes::ELM_GLOBAL_EXCALL, codes::ELM_EXCALL_ALLOC],
        &[],
    );
    command(
        &mut vm,
        vec![
            codes::ELM_GLOBAL_EXCALL,
            codes::ELM_EXCALL_FRONT,
            codes::STAGE_ELM_OBJECT,
            codes::OBJECTLIST_RESIZE,
        ],
        &[9],
    );
    assert_eq!(
        query(
            &mut vm,
            vec![
                codes::ELM_GLOBAL_EXCALL,
                codes::ELM_EXCALL_FRONT,
                codes::STAGE_ELM_OBJECT,
                codes::OBJECTLIST_GET_SIZE
            ]
        ),
        9
    );
    assert_eq!(
        query(
            &mut vm,
            vec![
                codes::ELM_GLOBAL_FRONT,
                codes::STAGE_ELM_OBJECT,
                codes::OBJECTLIST_GET_SIZE
            ]
        ),
        5
    );
    command(
        &mut vm,
        vec![
            codes::ELM_GLOBAL_FRONT,
            codes::STAGE_ELM_OBJECT,
            codes::OBJECTLIST_RESIZE,
        ],
        &[3],
    );
    assert_eq!(
        query(
            &mut vm,
            vec![
                codes::ELM_GLOBAL_EXCALL,
                codes::ELM_EXCALL_FRONT,
                codes::STAGE_ELM_OBJECT,
                codes::OBJECTLIST_GET_SIZE
            ]
        ),
        9
    );
}

#[test]
fn aliases_honor_runtime_stage_and_element_constants() {
    let mut vm = fixture(&[]);
    vm.ctx.ids.form_global_stage = 2000;
    vm.ctx.ids.elm_array = -9;
    vm.ctx.ids.stage_elm_object = 202;
    seed(&mut vm);
    for (alias, _, count) in STAGES {
        assert_eq!(
            query(&mut vm, vec![alias, 202, codes::OBJECTLIST_GET_SIZE]),
            count
        );
    }
    assert!(
        !vm.ctx
            .globals
            .stage_forms
            .contains_key(&codes::FORM_GLOBAL_STAGE)
    );
}

#[test]
fn malformed_and_unknown_alias_commands_remain_errors() {
    for chain in [
        vec![codes::ELM_GLOBAL_FRONT, codes::STAGE_ELM_OBJECT, 999_999],
        vec![codes::ELM_GLOBAL_FRONT, codes::STAGE_ELM_OBJECT, -1],
        vec![codes::ELM_GLOBAL_FRONT, 999_999],
    ] {
        let mut vm = fixture(&[]);
        assert!(
            vm.exec_command(chain, 0, vm.cfg.fm_int, &mut vec![])
                .is_err()
        );
    }
}
