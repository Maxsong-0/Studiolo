//! 副本上的核对和活的写者上的一样严（BUG-050）：同一组坏样本两边都拒，副本自己被动过手脚也拒。
use super::*;
use serde_json::Value;

const REQUESTS: &str = "history/meta-v1/requests";
const ACKS: &str = "file-operations-v1/acks";

/// 目录里名字以 `suffix` 结尾的唯一一个文件。
fn one(dir: PathBuf, suffix: &str) -> PathBuf {
    let mut found = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_str().unwrap().ends_with(suffix));
    let path = found.next().unwrap();
    assert!(found.next().is_none(), "不止一个 {suffix}");
    path
}
fn edit(path: &Path, change: impl FnOnce(&mut Value)) {
    let mut value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    change(&mut value);
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}
/// 重放目录里那条正文的终态（另一条是文件操作的）。
fn content_replay(data: &Path) -> PathBuf {
    fs::read_dir(data.join("idempotency"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            value["format_version"] == 4
        })
        .unwrap()
}
/// 换成另一个内容一样的目录：里面的文件还是原来那些，目录自己的 inode 变了。
fn swap(dir: &Path) {
    let aside = dir.with_extension("aside");
    fs::rename(dir, &aside).unwrap();
    fs::create_dir(dir).unwrap();
    for entry in fs::read_dir(&aside).unwrap() {
        let entry = entry.unwrap();
        fs::rename(entry.path(), dir.join(entry.file_name())).unwrap();
    }
    fs::remove_dir(aside).unwrap();
}
fn bump(value: &mut Value) {
    *value = Value::from(value.as_u64().unwrap() + 1);
}

/// 冻结之前写者目录里就坏了：定格不读回执、照样成；活的写者上核、副本上核，两边都拒，副本没有票据。
#[test]
fn the_copy_refuses_every_bad_record_the_live_writer_refuses() {
    type Plant = fn(&Path);
    let samples: [(&str, Plant); 13] = [
        ("正文回执的哈希被改", |data| {
            edit(&one(data.join(REQUESTS), "-intent.json.ack"), |ack| {
                ack["record_hash"] = "b".repeat(64).into();
            });
        }),
        ("正文回执的偏移被改成 0", |data| {
            edit(&one(data.join(REQUESTS), "-outcome.json.ack"), |ack| {
                ack["offset"] = 0.into();
            });
        }),
        ("正文回执的事件摘要被改", |data| {
            edit(&one(data.join(REQUESTS), "-outcome.json.ack"), |ack| {
                ack["digest"] = "b".repeat(64).into();
            });
        }),
        ("正文回执缺一份", |data| {
            fs::remove_file(one(data.join(REQUESTS), "-intent.json.ack")).unwrap();
        }),
        ("多出一份没有母本的正文回执", |data| {
            let ack = one(data.join(REQUESTS), "-intent.json.ack");
            let orphan = format!("{}-intent.json.ack", "0".repeat(64));
            fs::copy(&ack, ack.with_file_name(orphan)).unwrap();
        }),
        ("正文请求的描述被改", |data| {
            let ack = one(data.join(REQUESTS), "-intent.json.ack");
            let name = ack.file_name().unwrap().to_str().unwrap().to_owned();
            let id = name.strip_suffix("-intent.json.ack").unwrap();
            edit(&ack.with_file_name(format!("{id}.json")), |request| {
                bump(&mut request["bytes"]);
            });
        }),
        ("重放的指纹被改", |data| {
            edit(&content_replay(data), |replay| {
                replay["fingerprint"] = "b".repeat(64).into();
            });
        }),
        ("重放缺一份", |data| {
            fs::remove_file(content_replay(data)).unwrap();
        }),
        ("文件操作回执缺一份", |data| {
            fs::remove_file(one(data.join(ACKS), "-outcome.json")).unwrap();
        }),
        ("文件操作回执的哈希被改", |data| {
            edit(&one(data.join(ACKS), "-intent.json"), |ack| {
                ack["hash"] = "b".repeat(64).into();
            });
        }),
        ("多出一份文件操作回执", |data| {
            let ack = one(data.join(ACKS), "-intent.json");
            let extra = format!("{}-intent.json", "0".repeat(64));
            fs::copy(&ack, ack.with_file_name(extra)).unwrap();
        }),
        ("文件操作请求的序号被改", |data| {
            let record = one(data.join("file-operations-v1/records"), ".json");
            edit(&record, |request| bump(&mut request["sequence"]));
        }),
        (
            "文件操作的请求和回执都没了，重放还在",
            |data| {
                fs::remove_file(one(data.join("file-operations-v1/records"), ".json")).unwrap();
                for entry in fs::read_dir(data.join(ACKS)).unwrap() {
                    fs::remove_file(entry.unwrap().path()).unwrap();
                }
            },
        ),
    ];
    for (name, plant) in samples {
        let fixture = Fixture::new();
        let frozen = full_history(&fixture).freeze_checkpoint(deadline());
        plant(&fixture.data);
        both_refuse(&fixture, frozen, name);
    }
}

/// 定格成了、导出也成了，两边核的时候都拒。
fn both_refuse(fixture: &Fixture, mut frozen: FrozenBroker, name: &str) {
    assert!(
        verify_live(&frozen.broker, verify).is_err(),
        "活的写者没拒：{name}"
    );
    frozen.capture().unwrap_or_else(|e| panic!("{name}：{e}"));
    let exported = frozen
        .take_capture()
        .unwrap()
        .export(
            &fixture.target("left"),
            &fixture.target("right"),
            deadline(),
        )
        .unwrap_or_else(|e| panic!("{name}：{e}"));
    assert!(
        exported.verify(deadline(), verify).is_err(),
        "副本没拒：{name}"
    );
}

/// 维护槽的回执被改：正开着的写者在内存里记着槽的摘要，窗口里查水位就拒；改完才打开的写者记的就是坏的那一份，
/// 窗口里看不出来，两边都得在核回执的时候拒。
#[test]
fn a_changed_maintenance_receipt_is_refused_in_the_window_or_by_both_verifications() {
    let plant = |data: &Path| {
        edit(&data.join("maintenance-v1/slot.json"), |slot| {
            slot["record"]["outcome_ack"]["hash"] = "b".repeat(64).into();
        });
    };
    let fixture = Fixture::new();
    let mut frozen = full_history(&fixture).freeze_checkpoint(deadline());
    plant(&fixture.data);
    assert!(verify_live(&frozen.broker, verify).is_err());
    assert!(frozen.capture().is_err());

    let fixture = Fixture::new();
    drop(full_history(&fixture));
    plant(&fixture.data);
    let reopened = FsVaultBroker::open_existing(&fixture.vault, &fixture.data).unwrap();
    both_refuse(
        &fixture,
        reopened.freeze_checkpoint(deadline()),
        "维护槽的回执被改",
    );
}

/// 导出之后副本被动过：绑定文件、目录、笔记库标识、水位和定格那一刻对不上，都不出票。
#[test]
fn a_copy_that_no_longer_matches_the_capture_gets_no_ticket() {
    type Tamper = fn(&Path, &Path);
    let samples: [(&str, Tamper); 17] = [
        ("笔记库侧绑定", |vault, _| {
            edit(&vault.join(".workbench/broker-binding.json"), |b| {
                bump(&mut b["data"]["ino"]);
            });
        }),
        ("数据侧绑定", |_, data| {
            edit(&data.join("broker-binding.json"), |b| {
                bump(&mut b["canonical"]["ino"]);
            });
        }),
        ("笔记库标识", |vault, _| {
            edit(&vault.join(".workbench/vault.json"), |marker| {
                marker["vault_uuid"] = crate::VaultUuid::new().to_string().into();
            });
        }),
        ("笔记库标识被换成另一个文件", |vault, _| {
            let marker = vault.join(".workbench/vault.json");
            let copy = marker.with_extension("copy");
            fs::copy(&marker, &copy).unwrap();
            fs::rename(copy, marker).unwrap();
        }),
        ("历史目录被换", |_, data| swap(&data.join("history"))),
        ("重放目录被换", |_, data| {
            swap(&data.join("idempotency"))
        }),
        ("正文元数据目录被换", |_, data| {
            swap(&data.join("history/meta-v1"));
        }),
        ("请求目录被换", |_, data| swap(&data.join(REQUESTS))),
        ("正文元数据绑定", |_, data| {
            edit(&data.join("history/meta-v1/binding.json"), |b| {
                bump(&mut b["requests"]["ino"]);
            });
        }),
        ("文件操作目录被换", |_, data| {
            swap(&data.join("file-operations-v1"));
        }),
        ("维护目录被换", |_, data| {
            swap(&data.join("maintenance-v1"))
        }),
        ("文件操作绑定", |_, data| {
            edit(&data.join("file-operations-v1/binding.json"), |b| {
                bump(&mut b["quarantine"]["ino"]);
            });
        }),
        ("文件操作回执目录被换", |_, data| {
            swap(&data.join(ACKS))
        }),
        ("删除隔离目录被换", |vault, _| {
            swap(&vault.join(".workbench/deleted-objects"));
        }),
        ("维护绑定", |_, data| {
            edit(&data.join("maintenance-v1/binding.json"), |b| {
                bump(&mut b["root"]["ino"]);
            });
        }),
        ("水位多一笔", |_, data| {
            edit(&data.join("writer-state.json"), |state| {
                bump(&mut state["committed_through"]);
            });
        }),
        ("水位少一笔", |_, data| {
            edit(&data.join("writer-state.json"), |state| {
                let committed = state["committed_through"].as_u64().unwrap();
                state["committed_through"] = (committed - 1).into();
            });
        }),
    ];
    // 没动过的先核得过：下面每一条的失败都只能是那一处改动带来的。
    let untouched = |tamper: Option<Tamper>| {
        let fixture = Fixture::new();
        let mut frozen = full_history(&fixture).freeze_checkpoint(deadline());
        let (left, right) = (fixture.target("left"), fixture.target("right"));
        frozen.capture().unwrap();
        let exported = frozen
            .take_capture()
            .unwrap()
            .export(&left, &right, deadline())
            .unwrap();
        if let Some(tamper) = tamper {
            tamper(&left, &right);
        }
        exported.verify(deadline(), verify)
    };
    assert_eq!(untouched(None).unwrap().1.checked(), 6);
    for (name, tamper) in samples {
        assert!(untouched(Some(tamper)).is_err(), "没拒：{name}");
    }
}

/// 整个目标目录被换成另一个：句柄里读到的还是原来那一份，但路径上已经不是它了，不出票。
#[test]
fn a_target_replaced_after_the_export_gets_no_ticket() {
    let fixture = Fixture::new();
    let mut frozen = full_history(&fixture).freeze_checkpoint(deadline());
    let (left, right) = (fixture.target("left"), fixture.target("right"));
    frozen.capture().unwrap();
    let exported = frozen
        .take_capture()
        .unwrap()
        .export(&left, &right, deadline())
        .unwrap();
    fs::rename(&right, fixture.temp.path().join("right-old")).unwrap();
    fixture.target("right");
    assert!(exported.verify(deadline(), verify).is_err());
}

/// 核对也受截止时间约束：到期就停，不出票。
#[test]
fn an_expired_deadline_stops_the_verification() {
    let fixture = Fixture::new();
    let mut frozen = full_history(&fixture).freeze_checkpoint(deadline());
    frozen.capture().unwrap();
    let exported = frozen
        .take_capture()
        .unwrap()
        .export(
            &fixture.target("left"),
            &fixture.target("right"),
            deadline(),
        )
        .unwrap();
    assert!(exported.verify(Instant::now(), verify).is_err());
}

/// 账本那一侧不认（回调拒绝）就没有票据；每一条回执都问过，没有被跳过的。
#[test]
fn every_receipt_in_the_copy_is_put_to_the_ledger() {
    for refused in 1..=6 {
        let fixture = Fixture::new();
        let mut frozen = full_history(&fixture).freeze_checkpoint(deadline());
        frozen.capture().unwrap();
        let exported = frozen
            .take_capture()
            .unwrap()
            .export(
                &fixture.target("left"),
                &fixture.target("right"),
                deadline(),
            )
            .unwrap();
        let result = exported.verify(deadline(), |_, offset, _| {
            if offset == refused {
                return Err(BrokerError::CorruptWriterState);
            }
            Ok(())
        });
        assert!(result.is_err(), "第 {refused} 条没问账本");
    }
}

/// 复核也比目录身份：目录换成另一个、里面的文件一个没动，也算定格之后变了。
#[test]
fn a_directory_swapped_after_the_capture_fails_the_recheck() {
    let fixture = Fixture::new();
    let frozen = full_history(&fixture).freeze_checkpoint(deadline());
    let data = &frozen.broker.data;
    let captured = capture::capture(
        data,
        DATA_QUARANTINE,
        &mut capture::Budget::from_process(),
        deadline(),
    )
    .unwrap();
    captured.matches(data, DATA_QUARANTINE, deadline()).unwrap();
    swap(&fixture.data.join("history"));
    assert!(matches!(
        captured.matches(data, DATA_QUARANTINE, deadline()),
        Err(BrokerError::CaptureChanged)
    ));
}

/// 一个副本节点只能对应一个原件：拷乱了（同一个副本记两次）整份失败，没记过的不认。
#[test]
fn a_copy_node_stands_for_exactly_one_original() {
    let fixture = Fixture::new();
    let broker = fixture.open();
    let identity = crate::writer::Identity::directory(&broker.data).unwrap();
    let mut originals = super::super::exported::Originals::default();
    assert!(matches!(
        originals.of(identity),
        Err(BrokerError::WriterBindingMismatch)
    ));
    originals.record(identity, identity).unwrap();
    assert_eq!(originals.of(identity).unwrap(), identity);
    assert!(matches!(
        originals.record(identity, identity),
        Err(BrokerError::CaptureChanged)
    ));
}
