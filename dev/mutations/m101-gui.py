# M101's GUI half: `key_base_char' lives in a source file's own
# `#[cfg(test)]' block, which is a --lib target, not a --test one.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m101-gui.py \
#         -p frontend-gui --test-target lib
#
# R6 is the reviewer's SURVIVED prediction: the three new unit tests only ever
# construct `Key::Equals'/`Key::Minus', never `Key::Plus'.

PACKAGE = "frontend-gui"
TEST_TARGET = "lib"

MUTATIONS = [
    {
        "label": "R5 Equals arm has its shifted and unshifted characters swapped",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        K::Equals => if shift { '+' } else { '=' },",
        "new": "        K::Equals => if shift { '=' } else { '+' },",
    },
    {
        "label": "R6 the Plus arm maps the numpad plus key to '='",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        K::Plus => '+',",
        "new": "        K::Plus => '=',",
    },
]
