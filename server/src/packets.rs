use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(tag = "packet_type", content = "packet_data")]
pub enum ServerBoundPackets {
    ChoosePack {},
    JoinGame {
        code: String,
    },
    CreateGame {
        code: String,
        password: String,
        max_players: u8,
    },
}

#[derive(Serialize)]
#[serde(tag = "packet_type", content = "packet_data")]
pub enum ClientBoundPackets {
    PackResponse { accepted: bool },
    RegisterPack,
}
