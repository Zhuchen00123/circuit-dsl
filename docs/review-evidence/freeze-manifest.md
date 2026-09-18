# 最终冻结清单（本轮交付的权威基线）

git HEAD: cb5d8a212f66922181580900a05fb3d42abe32f2（工作区未提交；按用户要求不 commit、不 push）

## 0. 冻结历史与漂移披露（回应代码审核 a12 的 F1、最终门禁 a15 的 G1）

1. **F0 代码冻结**（第一版清单，16 项）：发生在本轮实现写完之后。
2. **F0 后写入**：文档同步代理写了 `docs/backend-evaluation.md`、`docs/testing.md`、`examples/rc_filter.cdsl`（不在 F0 清单内）；
   a12 指出覆盖缺口后新增 `crates/circuit-dsl/tests/phase_syntax_regression.rs`；Lead 随后修正 `docs/architecture.md` 与 README 的收尾数字。
3. a12 的 F1（P2）：仅凭 F0 清单无法发现这些漂移 —— **采纳**，重新冻结为第二版（30 项）。
4. a15 的 G1（P2）：第二版清单的 `README.md` 与 `implementation-summary.md` 哈希在其生成之后又被写入。
   —— **采纳**：本版（第三版，最终版）为**所有写入停止之后**重新计算，共 32 项，全部与工作区一致。
   第二版清单中 README 的漂移内容已由 a15 用快照重建核对：是正向更正（389→413、旧理想阶跃归因→已验证的 6025 点 / 4.999167e-7 V），不含代码/测试改动。
5. 自引用盲点：本清单不包含自身（`freeze-manifest.md`）的哈希；原始日志目录 `raw-*.txt` 与 `cli-qa-output/` 也不参与哈希。

## 1. 文件哈希（SHA-256，全部在工作区写入停止后计算）

| 文件 | SHA-256 |
|---|---|
| `crates/circuit-core/src/connectivity.rs` | A30538656DF2423BFB28543DB52BA47B18BBCD9010D1436EDD4C450CC68220DB |
| `crates/circuit-dsl/src/elaborate.rs` | F95ECB7793F69C6D97995D5BA6D34B37D0E7914BAB9444B5A3AC4D7500B3C794 |
| `crates/circuit-backend/src/thevenin.rs` | CCB30627A0A5872FBE497D212ED9774F7DEAF8AFB6231DF3944C40081466848E |
| `crates/circuit-cli/tests/e2e.rs` | 5FA60B3A4E24A981B3AB61881BAA42EB89253794B5CAFE7171CE519512738263 |
| `_probe/src/main.rs` | 4E58AAE5A021BB7ED350867F31123EC74A6820C94136B88076B7CB65FB1DE0A2 |
| `_probe/src/bin/robustness.rs` | BD52BC831BC869B80AD813DC8C6295E5C1A51568009845933C392A4926FA9FBB |
| `_probe/Cargo.toml` | 96852193E4623F2B6BBC1FB88372D2CE171358650B93C9B53716853FFFFD230C |
| `crates/circuit-dsl/tests/reference_path_regression.rs` | 863FBFD512EBC32C1E63EA7C1DF8F0B3ABEDD37379A49299596CE0E25737C16B |
| `crates/circuit-dsl/tests/phase_syntax_regression.rs` | E2470F86C647101D63CC678230783C96D6D0012B48CF5FC625EC6A6D64EF70CB |
| `crates/circuit-backend/tests/transient_reference_regression.rs` | E147AF1259FC8E32FCC81893A2C67DE029364F1595ECABC43937BDA2EF21AACF |
| `crates/circuit-backend/tests/phase_regression.rs` | F4CBD7DAE5F31923DEE08449D2BE4B1D2426E880DEC5971A24CFDCD73EE82367 |
| `crates/circuit-backend/tests/adapter.rs` | 6AE48B621274BA826F97B4EAED193449D535692DD1D24836A3518BB7C1E494D0 |
| `crates/circuit-dsl/tests/elaborate.rs` | 5D5F94DCA7860064CAAD7488E1EFC6AA8416D575D1AA3C95E739814445FE213E |
| `README.md` | C832D08EF2B08C813E749D8003FF133DD4358118C97192F45729467F290F8C3A |
| `docs/architecture.md` | AB4B5CDC998614F0CBBD2E6E373C713E3B0BA6FD5BA50040C2C5DF73F246341E |
| `docs/language.md` | 730A3D36F027CC921B26CF306D16F10DC3B52BDB0CFE79FDBD8B0EF2B518CC62 |
| `docs/backend-evaluation.md` | BEFB3ECE42218670C4983F6371F7BE20AF4B9FFD400AC376726F9B1497A75B7F |
| `docs/testing.md` | AFF67E8E903AF7D8EC52C8DDDDE2EBE8EE59E8994496AC73D3E0A82D27A16434 |
| `examples/rc_filter.cdsl` | DD537CBA219E3044D921906AF24EFF1F9BE97934B0F0247BADFABA6312875267 |
| `docs/review-evidence/team-board.md` | D247F3960FEABD89ACCE38CF0E50EF056E2C269BE6422375967A71CF3D3B7107 |
| `docs/review-evidence/baseline.md` | 93957141A71E5FB39A737C34DA054BF22121449CF0698527DF7E35CC1E71FD25 |
| `docs/review-evidence/repo-forensics.md` | 922BD9E1AC3A5DD7B69D579F18B77E09CA8105194051A7944438F861ADA92CDE |
| `docs/review-evidence/floating-audit.md` | A583589B6795B54AF3B663105E21E631119EC7428D7F3585AEEEFFDDFC098FB7 |
| `docs/review-evidence/rc-reference-math.md` | 7268F64450D9462C4ED7C6677B4378CE35D576A7506D186197E56104521D293A |
| `docs/review-evidence/backend-contract.md` | 20A78C235ED29A4F4B3A73CC815CA46508DDEBBA5F47FBBDEFA8EA4C4C34AB46 |
| `docs/review-evidence/next-round-contracts.md` | EC63540C7C59F4D13D54AF889389C52F44A1E61E0997DF074B59D87AB8CC68B5 |
| `docs/review-evidence/numerical-review.md` | BEA7904519F1BEE4EFD2A70FFD2FD401664A288E6D3A62FEFBC5D4F75807604E |
| `docs/review-evidence/code-review.md` | 6FC4E6EDAE25EAEF89AA51EE58366192104A3DC8BD4EEBD70B22A56EC9934A93 |
| `docs/review-evidence/cli-qa.md` | 7E53C48C15F246CCFFFA80CCE41C29E3BBA1F9C77639718A4895DDE821895BC9 |
| `docs/review-evidence/final-gate.md` | FC541D529AB94C99752BED1983F4F74A0D1B11A16AE8AD140C1A6556B65E50E4 |
| `docs/review-evidence/implementation-summary.md` | 0719EDC7102077E96BB26441B0E3B05D90C9AE96A37C3F841D873F50CE16B99D |
| `docs/review-evidence/final-summary.md` | 6A1E45FF43DE1D84D77507BBC1745386639CE4A7ACBE3D308397564AA832B156 |

## 2. 最终门禁（Lead 在本清单对应的最终树上实跑，全部 exit 0）

```text
cargo test --workspace                                exit 0   413 passed / 0 failed / 0 ignored
cargo clippy --workspace --all-targets -- -D warnings  exit 0
cargo fmt --all -- --check                             exit 0
cargo run --manifest-path _probe/Cargo.toml --bin probe       exit 0（6/6 Case PASS）
cargo run --manifest-path _probe/Cargo.toml --bin robustness  exit 0（13/13 子用例 PASS）
cargo fmt --manifest-path _probe/Cargo.toml -- --check        exit 0
```

原始输出：`docs/review-evidence/raw-final-workspace-{test,clippy,fmt}.txt`、
`target/lead-qa/probe/{probe-final,robustness-final,fmt}.txt`（`target/` 不入版本管理）。

历史基线 389 passed（开工实测一致）→ 新增 24 个测试（8 + 4 + 6 + 6）→ **413 passed**。

## 3. 门禁计数口径
20 条 `test result:` 行 = 14 个测试二进制 + 5 个 doc-test 目标 + 1 条额外行；
只有 `circuit-results` 的 doc-test 含 1 个测试，非零计数共 15 行，求和 413。

## 4. 审核结论对照
- 数值复核 `numerical-review.md`：**PASS**（独立复算全部数字一致；判据未放宽、无筛样本、无循环论证）。
- 代码审核 `code-review.md`：**PASS（代码/测试）**；F1 已由本清单三版冻结闭环；F2 已修正；F3 已修正；
  **F4 经 Lead 与 a15 各自独立复核判定为不成立（误报）**：`thevenin.rs:772-775` 确为 `let step = spec.output_interval…` 块，
  `tmax: spec.max_step` 在 `:781`；该文件哈希与冻结值一致。
- 最终门禁 `final-gate.md`：**PASS**（6/6 门禁、413 一致、13 个代码/测试文件与清单一致、五套证据无矛盾）。
- CLI QA `cli-qa.md`：**PASS** + 2 条 LOW（退出码 2 不可达已入文档；`--out` 指向文件时的报错路径已记为限制）。