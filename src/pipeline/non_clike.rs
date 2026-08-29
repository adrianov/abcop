//! Trait unifying every directive-aware non-clike backend behind one
//! three-check protocol so dispatch stays a single typed call per arm.

use tree_sitter::Tree;

use crate::abc::AbcOffense;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;
use crate::{csharp, dart, golang, javalang, phplang, pylang, sollang};

pub(super) trait NonClike {
    type Model<'a>;
    fn build<'t>(src: &'t [u8], tree: Tree) -> Self::Model<'t>;
    fn all_scores(model: &Self::Model<'_>) -> Vec<AbcOffense>;
    fn used_once_offenses(model: &Self::Model<'_>) -> Vec<UsedOnceOffense>;
    fn never_used_offenses(model: &Self::Model<'_>) -> Vec<NeverUsedOffense>;
}

macro_rules! non_clike_backend {
    ($marker:ident, $module:ident, $model:ident) => {
        pub(super) struct $marker;

        impl NonClike for $marker {
            type Model<'a> = $module::$model<'a>;

            fn build<'t>(src: &'t [u8], tree: Tree) -> Self::Model<'t> {
                $module::build(src, tree)
            }

            fn all_scores(model: &Self::Model<'_>) -> Vec<AbcOffense> {
                $module::abc::all_scores(model)
            }

            fn used_once_offenses(model: &Self::Model<'_>) -> Vec<UsedOnceOffense> {
                $module::used_once_offenses(model)
            }

            fn never_used_offenses(model: &Self::Model<'_>) -> Vec<NeverUsedOffense> {
                $module::never_used_offenses(model)
            }
        }
    };
}

non_clike_backend!(PyB, pylang, PyFile);
non_clike_backend!(GoB, golang, GoFile);
non_clike_backend!(PhpB, phplang, PhpFile);
non_clike_backend!(JavaB, javalang, JavaFile);
non_clike_backend!(CSharpB, csharp, CSharpFile);
non_clike_backend!(SolidityB, sollang, SolFile);
non_clike_backend!(DartB, dart, DartFile);
