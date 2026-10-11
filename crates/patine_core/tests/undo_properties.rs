//! Property tests of the history: any sequence of commands, undos and redos, followed by
//! undoing everything, gives back the initial document; redoing everything gives back
//! the final one. Every intermediate document is valid.

mod common;

use common::ops::{Op, command, op};
use patine_core::history::History;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn undo_everything_restores_the_initial_document(ops in proptest::collection::vec(op(), 1..80)) {
        let initial = common::project();
        let mut project = initial.clone();
        let mut history = History::default();
        for op in &ops {
            match op {
                Op::Seal => history.seal(),
                Op::Undo => {
                    history.undo(&mut project).expect("undo");
                }
                Op::Redo => {
                    history.redo(&mut project).expect("redo");
                }
                _ => {
                    if let Some(command) = command(&project, op) {
                        let before = project.clone();
                        if history.push(&mut project, command).is_err() {
                            prop_assert_eq!(&project, &before, "a refused command changed the document");
                        }
                    }
                }
            }
            prop_assert_eq!(project.validate(), Ok(()), "after {:?}", op);
        }

        let final_document = project.clone();
        let applied = history.applied();
        history.go_to(&mut project, 0).expect("undo everything");
        prop_assert_eq!(&project, &initial);
        prop_assert!(!history.is_dirty());
        history.go_to(&mut project, applied).expect("redo everything");
        prop_assert_eq!(&project, &final_document);
    }
}
