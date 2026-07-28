//! Authored and extracted evidence attached to title-boundary transitions.

use super::*;

pub(super) fn reset_rule_evidence() -> RuleEvidence {
    RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![
                EvidenceRecord {
                    id: "binary.gz2e01.dcomifg-reset-to-opening".into(),
                    kind: EvidenceKind::Extracted,
                    source_sha256: Some(parse_digest(
                        "bde63a102b6502e418e5a8c53cff364f66f6510420a7316a492664ab7530e28d",
                    )),
                    note: "Canonical exact-DOL function artifact: VA 0x8002cd44, size 0x74, code SHA-256 3cc637771d531950401a332a83b90296df2b5aa9bec6cc292ad5546fec23df30.".into(),
                },
                EvidenceRecord {
                    id: "binary.gz2e01.dcomifg-change-opening-scene".into(),
                    kind: EvidenceKind::Extracted,
                    source_sha256: Some(parse_digest(
                        "658f63b09b0f43dcb5b2662dbbf140de889fe19374dac8ccee32d9545ac2d781",
                    )),
                    note: "Canonical exact-DOL function artifact: VA 0x8002cc54, size 0xf0, code SHA-256 0b5c465a32ffb343d9863e04970f5c2621a5bb0b854efc974708fb0229828a41.".into(),
                },
                EvidenceRecord {
                    id: "source.gcn-reset-to-opening-prefix".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(parse_digest(
                        "b9b37aed0b76eef2d27b35a2ece6ee077086a970f98d18936a83649303f15761",
                    )),
                    note: "Source-family audit establishes the GCN guards, F_SP102/start 100/room 0/layer 10 pending load, PROC_OPENING_SCENE request, and restart-room parameter zero write.".into(),
                },
            ],
        }
}

pub(super) fn scheduler_rule_evidence() -> RuleEvidence {
    RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![EvidenceRecord {
                id: "source.gz2e01-title-process-activation".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "f095894aabc198c068ee0ac9872f6c277c0e035b36c4d29d1f896e7c2eb0fe4b",
                )),
                note: "GZ2E01 process audit separates a submitted scene request from the scheduler-observed opening/name process create phase; these transitions record those independently observed activations.".into(),
            }],
        }
}

pub(super) fn opening_rule_evidence() -> RuleEvidence {
    RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![
                exact_function_evidence(
                    "binary.gz2e01.opening-phase-4",
                    "caf6f662835287e2c74e341b2771e142c8b0a1dd6da7745775a01f1a36cb62cc",
                    "phase_4__FP9dScnPly_c at VA 0x8025a654, size 0x3a0, code SHA-256 5e116171d689fcf368218490f24009dd176205648fd30b697bdab3a7efb179aa.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.dsv-info-init",
                    "433224e88c9c58df6d5abd49863e2a871a965f2806288e1d19fd36f1e267d93b",
                    "init__10dSv_info_cFv at VA 0x80034fcc, size 0x50, code SHA-256 5c80b3dba87ae8f968b5e4620f0872d4355358debc63d5556adba4b8d3d4338d.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.dsv-player-init",
                    "0bc0b6246b3a6cad9a8a0409ef59358fa544632ac5884b27008a3e5dd4db185b",
                    "init__12dSv_player_cFv at VA 0x800346a4, size 0xac, code SHA-256 668f452c16c5ed413535588b00c5a497b236a29f7e52f55c521b58e179968766.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.dsv-save-init",
                    "a9953253f543fbdc9d0998e6f369fb2f0bac45b411c44baee5ff9fd34fccda9b",
                    "init__10dSv_save_cFv at VA 0x8003501c, size 0x8c, code SHA-256 e405d830e4f445c950fb158ddf8f6107430524a2708d82bd1b31c7e13e804d48.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.empty-initial-event-hook",
                    "c40daaee608a8afd5c471d54a1a87efe7eb42695036729215a3fa413d256892f",
                    "setInitEventBit__Fv at VA 0x80035c88 is an exact four-byte immediate return, code SHA-256 f332ea5b5437103cbb6f1508679da89eec9288ad775c96c439a17fccabe3de8e.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.player-return-place-init",
                    "0eeb93826008824d6810499ce61ec1c8e8065c7a06c8a9576022b76532f75917",
                    "init__25dSv_player_return_place_cFv at VA 0x80032cc8, size 0x54, code SHA-256 252007ca2690e54e6a13019527739c4e55dff0f1ac1e7ec6ff8b1d425ed6ab87.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.select-equip-shield",
                    "7a7920012416bdf116d20be436514da59bf00da2e6cbab28dcc0842e33078a23",
                    "dComIfGs_setSelectEquipShield__FUc at VA 0x8002ef94, size 0xac, code SHA-256 beeb64d1fa6897f83de2674e9053189416486ca4066c39d1efb4e647bf7c7e14.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.select-equip-sword",
                    "1d014bd60aa88951beb555a13853be0068f91790989639909bcff8a088decd9e",
                    "dComIfGs_setSelectEquipSword__FUc at VA 0x8002eec0, size 0xd4, code SHA-256 b0cdfc30b3f91a906cf4c8066f8eb5ec7055df50de7ade590c5c721ea0732761.",
                ),
                EvidenceRecord {
                    id: "source.gz2e01.opening-file0-initialization".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(parse_digest(
                        "c8f30a83c45d6c42078945b09f6e4e3459c832184e641ff442fa7d0e49258077",
                    )),
                    note: "Opening phase 4 initializes dSv_info, life, Kokiri clothes, Ordon sword, Hylian shield, and event 0x0601. Sword/shield setters set collection masks but off-item-bit=false leaves acquisition bits clear.".into(),
                },
                EvidenceRecord {
                    id: "source.gz2e01.save-domain-initializers".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(parse_digest(
                        "7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453",
                    )),
                    note: "dSv_info_c::init resets savedata, live stage memory, dungeon memory, zones, and temporary event state; nested player initialization establishes the exact retained fields published here.".into(),
                },
            ],
        }
}

pub(super) fn title_rule_evidence() -> RuleEvidence {
    RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![EvidenceRecord {
                id: "source.gz2e01.title-key-and-name-scene-request".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "39378bcbc78e5ffae3287f127cc48cd2c22e18723cf31cfeb5bd84a2becdc4cb",
                )),
                note: "GZ2E01 source audit: title keyWait accepts A/Start, advances to nextScene, and nextScene requests PROC_NAME_SCENE only while reset and overlap-peek are clear.".into(),
            }],
        }
}

pub(super) fn file_select_rule_evidence() -> RuleEvidence {
    RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![
                EvidenceRecord {
                    id: "source.gz2e01.name-scene-create".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(parse_digest(
                        "f095894aabc198c068ee0ac9872f6c277c0e035b36c4d29d1f896e7c2eb0fe4b",
                    )),
                    note: "GZ2E01 source audit: the normal name-scene create path constructs file select, then writes mNoFile = 0.".into(),
                },
                EvidenceRecord {
                    id: "source.gz2e01.file-select-create".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(parse_digest(
                        "aee1cb134ec92953fd04dc321f4dae5f5c98ed1d2e766d1306a70d932294eb0d",
                    )),
                    note: "GZ2E01 source audit: dFile_select_c::_create runs dComIfGs_init and then writes mNewFile = 0 before the name scene enters file-select-open.".into(),
                },
            ],
        }
}

pub(super) fn file_select_branch_rule_evidence() -> RuleEvidence {
    RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![
                EvidenceRecord {
                    id: "source.gz2e01.file-select-branches".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(parse_digest(
                        "aee1cb134ec92953fd04dc321f4dae5f5c98ed1d2e766d1306a70d932294eb0d",
                    )),
                    note: "GZ2E01 file-select audit separates blank-slot mNewFile/mDataNum writes, existing-slot Start/card_to_memory, and no-save buffer initialization/card_to_memory/header writes.".into(),
                },
                EvidenceRecord {
                    id: "source.gz2e01.card-to-memory".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(parse_digest(
                        "7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453",
                    )),
                    note: "dSv_info_c::card_to_memory copies dSv_save_c only, then performs load-time life/key/item-layout normalization; live header and other non-save runtime metadata are outside that projection.".into(),
                },
                exact_function_evidence(
                    "binary.gz2e01.card-to-memory",
                    "fca390c69693273eab6336a9ce094473227ea9c98f4e13a627c12452ddc12352",
                    "card_to_memory__10dSv_info_cFPci at VA 0x80035a04, size 0x1cc, code SHA-256 5f50141704f8daa60900f0559ef6f2272965b195fa673d29e73ceef82a593dc0.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.set-line-up-item",
                    "f9edd7f12fcbce48fb6c07b036ae3018abb07d8fb1510044f848a05eacbf7a14",
                    "setLineUpItem__17dSv_player_item_cFv at VA 0x800332f8, size 0x5c, code SHA-256 08c250dbed9821493d7a25ae234328a99fe912228b8ac54bcffe5314b5c1e323.",
                ),
            ],
        }
}

pub(super) fn name_confirmation_rule_evidence() -> RuleEvidence {
    RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![
                EvidenceRecord {
                    id: "source.gz2e01.file-select-name-confirmation".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(parse_digest(
                        "aee1cb134ec92953fd04dc321f4dae5f5c98ed1d2e766d1306a70d932294eb0d",
                    )),
                    note: "Source audit establishes both name confirmations, default horse setup, both Back paths, and final mIsSelectEnd. These mutate live dSv_save_c player-info; no physical save API is called.".into(),
                },
                exact_function_evidence(
                    "binary.gz2e01.file-select-name-input",
                    "fd93ea0a72e1008434af10c19cd8f59a430f01bd8a044f5173bd97e78bd6ae0a",
                    "nameInput__14dFile_select_cFv at VA 0x801873bc, size 0x13c, code SHA-256 0388366b478b3a51aa2a7cd4c7825eb7370dec67b14e3b7db98e2c9aad284ba5.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.file-select-name-input-fade",
                    "ecb601568e64364a3adfc779bf737949371a1460c1daca3651ec31ef1631c726",
                    "nameInputFade__14dFile_select_cFv at VA 0x8018759c, size 0x104, code SHA-256 1972401d18a34e1f1d8c6ab180df465df2c17d34a9fc03dbcdda37b1229249d8.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.file-select-name-input-2-move",
                    "9da639084fa4d342c1154c2669aa65eb22c81d3fa52b9281f0ab100c15a86f33",
                    "nameInput2Move__14dFile_select_cFv at VA 0x801876a0, size 0xac, code SHA-256 a96931c928651f29eea71bf214964abe46f8af5a7a3006581153fef732c614e5.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.file-select-name-input-2",
                    "e7a2a4b3ed67e42938aa0a28f2deaa66edab757618d0bcacdaef3598e627cc13",
                    "nameInput2__14dFile_select_cFv at VA 0x8018774c, size 0xd8, code SHA-256 32fb5e79113d0a52bde235fd8c1fb3c052b66445bc1b7264e8c065d53e5ea87b.",
                ),
            ],
        }
}

pub(super) fn successful_save_rule_evidence() -> RuleEvidence {
    RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![
                EvidenceRecord {
                    id: "source.gz2e01.save-menu-success".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(parse_digest(
                        "78acd5de6255c5031eeeb0d041509b9080b7121e68a1546d14ba75a6454f0f4e",
                    )),
                    note: "dMenu_save_c dataWrite commits the current stage, projects the selected entry, checksums it, and submits the full buffer. Only SaveSync result 1 updates mDataNum/mNoFile and enters a success UI branch.".into(),
                },
                EvidenceRecord {
                    id: "source.gz2e01.memory-to-card".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(parse_digest(
                        "7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453",
                    )),
                    note: "memory_to_card copies dSv_save_c after temporary lantern normalization, then restores the live lantern/event values. The promoted neutral branch proves those temporary transforms are identity on projected fields.".into(),
                },
                exact_function_evidence(
                    "binary.gz2e01.put-save",
                    "eb3032a28f0a4d08684d74894785c1760a241020d907b12bee19e350eda1caf9",
                    "putSave__10dSv_info_cFi at VA 0x800350f0, size 0x5c, code SHA-256 f94364f83aed527671a218a8e0a5b2a9e541578fbd775176981f22df31fddd6e.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.memory-to-card",
                    "5b65a8833c8fb246e5c0292e0f22ecf6b05f5e3a123f2f18ee33c343a9805f1e",
                    "memory_to_card__10dSv_info_cFPci at VA 0x80035798, size 0x26c, code SHA-256 7cf6fc958ed1e4cdcf4b3e168364cbd7a42a545a1812d139a4442e41ae5fd8e9.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.save-menu-data-write",
                    "cf1308d2ecb1741549ce173a76f7e7c0ff8fe7343156632baae499dea1836ebb",
                    "dataWrite__12dMenu_save_cFv at VA 0x801f2840, size 0xa4, code SHA-256 b6a30e6925392a2c876f0f002e93afeb257da6878b989515c12fe83b58c6ac35.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.save-menu-wait",
                    "8b8f2e635426fdd8dc3e4cf4c49953ef1518e6836dca669acbd5cd5706ad0394",
                    "memCardDataSaveWait__12dMenu_save_cFv at VA 0x801f28e4, size 0xa8, code SHA-256 ab833e5d0f988b09921e3788272ebaa325767f91f649af3209ff0bcff6b40778.",
                ),
                exact_function_evidence(
                    "binary.gz2e01.save-menu-wait-2",
                    "c0bdf0610b4b25b22ddf5dab9745bbf8dfdd8267d02daaf878186335eb3b1d88",
                    "memCardDataSaveWait2__12dMenu_save_cFv at VA 0x801f298c, size 0x1d0, code SHA-256 206affd3eccd29c55beed5853501307985d355504ab3c4d5ebbb076dd719022f.",
                ),
            ],
        }
}

pub(super) fn play_scene_request_rule_evidence() -> RuleEvidence {
    RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![
                EvidenceRecord {
                    id: "source.gz2e01.name-scene-change-game-scene".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(parse_digest(
                        "f095894aabc198c068ee0ac9872f6c277c0e035b36c4d29d1f896e7c2eb0fe4b",
                    )),
                    note: "dScnName_c::changeGameScene calls dComIfGs_gameStart, overrides a new file's next stage with F_SP108/room 1/spawn 21/layer 13, and requests PROC_PLAY_SCENE without proving process or world activation.".into(),
                },
                EvidenceRecord {
                    id: "source.gz2e01.game-start-return-place".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(parse_digest(
                        "b9b37aed0b76eef2d27b35a2ece6ee077086a970f98d18936a83649303f15761",
                    )),
                    note: "dComIfGs_gameStart requests the structured player return place with layer -1 before the new-file branch optionally overrides it.".into(),
                },
            ],
        }
}
