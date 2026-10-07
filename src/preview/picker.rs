//! ratatui-image's encoder, built for a protocol that is already decided.
//!
//! ratatui-image would rather query the terminal itself, and its querying constructor is the one it
//! recommends. Here the protocol comes from [`graphics::decide`](super::graphics::decide), which
//! distrusts a multiplexer's answers, and the cell size from [`probe`](super::probe), which reads
//! through the application's own input stream — so only the encoder is wanted, and it is built from
//! a cell size with the protocol forced. ratatui-image's kitty encoder places pictures with unicode
//! placeholders, which is what makes them survive scrolling and redraws.

use ratatui_image::FontSize;
use ratatui_image::picker::{Picker, ProtocolType};

use super::graphics::{CellSize, Protocol};

crate::provenance! {
    component: "preview::picker",
    about: "ratatui-image's encoder for a decided graphics protocol and cell size, and the encoded protocols it produces",
    origin: crate::Origin::Upstream("ratatui-image"),
    lineage: crate::Lineage::Wrapper {
        crate_name: "ratatui-image",
        req: "11.1",
    },
    since: "0.1",
}

/// An encoder for `protocol` at `cell` pixels per cell, or `None` for a protocol that draws no
/// pictures.
#[must_use]
pub fn picker(protocol: Protocol, cell: CellSize) -> Option<Picker> {
    let kind = match protocol {
        Protocol::Kitty => ProtocolType::Kitty,
        Protocol::Sixel => ProtocolType::Sixel,
        Protocol::Halfblocks => ProtocolType::Halfblocks,
        Protocol::None => return None,
    };
    // Deprecated upstream in favour of querying; the querying is done, and only the encoder is wanted.
    #[allow(deprecated)]
    let mut picker = Picker::from_fontsize(FontSize::new(cell.width, cell.height));
    picker.set_protocol_type(kind);
    Some(picker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_protocol_that_draws_gets_its_own_encoder() {
        let cell = CellSize {
            width: 8,
            height: 16,
        };
        for (protocol, kind) in [
            (Protocol::Kitty, ProtocolType::Kitty),
            (Protocol::Sixel, ProtocolType::Sixel),
            (Protocol::Halfblocks, ProtocolType::Halfblocks),
        ] {
            let encoder = picker(protocol, cell).expect("a protocol that draws has an encoder");
            assert_eq!(encoder.protocol_type(), kind);
            assert_eq!(encoder.font_size().width, 8);
            assert_eq!(encoder.font_size().height, 16);
        }
        assert!(picker(Protocol::None, cell).is_none());
    }
}
