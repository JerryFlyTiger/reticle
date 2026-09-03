// SPDX-License-Identifier: LicenseRef-FSL-1.1-ALv2
// Copyright 2026 Jerry Chen
//
// Reticle is source-available software, licensed under the Functional
// Source License 1.1 with an Apache 2.0 future grant. It is not open source.
// See LICENSE.md for the terms, and THIRD_PARTY_LICENSES.md for the licenses
// of the dependencies it links against.

//! Guards the window icon `lib.rs` embeds with `include_bytes!`. This does
//! not run any GUI code -- it decodes the same bytes the real binary embeds
//! and checks the shape `lib.rs` assumes (256x256), so the test fails if the
//! asset goes missing, gets corrupted, or is regenerated at a different
//! size, instead of that surfacing later as a silently absent window icon.

const ICON_PNG_BYTES: &[u8] = include_bytes!("../../../assets/icon/png/reticle-256.png");

#[test]
fn embedded_icon_decodes_to_the_expected_256_square() {
    let icon = eframe::icon_data::from_png_bytes(ICON_PNG_BYTES)
        .expect("assets/icon/png/reticle-256.png must decode as a valid PNG");

    assert_eq!(icon.width, 256, "icon width must stay 256");
    assert_eq!(icon.height, 256, "icon height must stay 256");
    assert_eq!(
        icon.rgba.len(),
        256 * 256 * 4,
        "decoded RGBA buffer must be exactly width * height * 4 bytes"
    );
}
