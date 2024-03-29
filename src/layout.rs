use rustkbd::keyboard::{self, layout, Key};

use crate::switches::SwitchIdentifier;

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Layout {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, keyboard::Layer)]
pub enum Layer {
    Default,
    Lower,
    Raise,
}

impl Default for Layer {
    fn default() -> Self {
        Self::Default
    }
}

impl Layout {
    const KEY_CODES_DEFAULT: [[Key; 12]; 4] = layout! {r"
        | Esc |  Q  |  W  |  E  |  R  |  T  |  Y  |  U  |  I  |  O  |  P  | Del |
        | LCtl|  A  |  S  |  D  |  F  |  G  |  H  |  J  |  K  |  L  |  ;  |  '  |
        | LSft|  Z  |  X  |  C  |  V  |  B  |  N  |  M  |  ,  |  .  |  /  |Enter|
        |     |     |     | LAlt| LGui|Space|     |     |     |     |     |     |
    "};
    const KEY_CODES_LOWER: [[Key; 12]; 4] = layout! {r"
        | Trn |  1  |  2  |  3  |  4  |  5  |  6  |  7  |  8  |  9  |  0  | Tab |
        | Trn |  !  |  @  |  (  |  )  |  *  |  -  |  =  |  [  |  ]  | Pipe|  `  |
        | Trn |  %  |  ^  |  #  |  $  |  &  |  _  |  +  |  {  |  }  |  \  |  ~  |
        |     |     |     | Trn | Trn | Trn |     |     |     |     |     |     |
    "};
    const KEY_CODES_RAISE: [[Key; 12]; 4] = layout! {r"
        | Trn |     |     |     |     |     |     |     |     |MVlDn|MMute|MVlUp|
        | Trn |     |     |     |     |     |     |     |     |     |  Up |     |
        | Trn |     |     |     |     |     |MPrev|MPlPs|MNext| Left| Down|Right|
        |     |     |     | Trn | Trn | Trn |     |     |     |     |     |     |
    "};
}

impl rustkbd::keyboard::Layout<2> for Layout {
    type Identifier = SwitchIdentifier;
    type Layer = Layer;

    fn layer(&self, switches: &[Self::Identifier]) -> Layer {
        switches
            .iter()
            .map(|switch| match switch {
                SwitchIdentifier { row: 3, col: 7 } => Layer::Lower,
                SwitchIdentifier { row: 3, col: 8 } => Layer::Raise,
                _ => Layer::Default,
            })
            .max()
            .unwrap_or_default()
    }

    fn key(&self, layer: Layer, switch: &Self::Identifier) -> Key {
        match (layer, *switch) {
            (Layer::Default, SwitchIdentifier { row, col }) => {
                Self::KEY_CODES_DEFAULT[row as usize][col as usize]
            }
            (Layer::Lower, SwitchIdentifier { row, col }) => {
                Self::KEY_CODES_LOWER[row as usize][col as usize]
            }
            (Layer::Raise, SwitchIdentifier { row, col }) => {
                Self::KEY_CODES_RAISE[row as usize][col as usize]
            }
        }
    }
}
