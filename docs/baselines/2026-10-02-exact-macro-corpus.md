# 公开宏语料最小纵向流程

日期：2026-10-02。范围：公开读取、清洗、家族划分、一个原生兼容案例。本宏库流程未训练学习模型，未接入生产合成请求，未上传语料或部署。其他模块使用项目构造短例产生的学习样本不属于本宏库规模。

## 读取路径与边界

[JX3BOX 宏库入口](https://www.jx3box.com/macro/)的普通文本读取只返回浏览器兼容提示。检查该页面实际加载的公开脚本后，确认当前读取路径：

| 证据 | 观察到的用途 |
| --- | --- |
| [当前宏栏目服务脚本](https://cdn.jx3box.com/static/pve/js/1802.1361821e.js) | 公开列表 `GET /api/cms/posts`，公开正文 `GET /api/cms/post/{id}` |
| [当前列表脚本](https://cdn.jx3box.com/static/pve/js/2388.7c731806.js) | `type=macro`，`subtype`、`client`、`page`、`per`、`order=update`；选择心法时 `sticky=1` |
| [当前详情脚本](https://cdn.jx3box.com/static/pve/js/1436.5e89f93a.js) | 正文取 `post_content`，独立宏块取 `post_meta.data[].macro` |

实际 API 域名为 `https://cms.jx3box.com`，本轮匿名 GET 已取得列表与正文。以上路径来自当日实际加载脚本及成功响应，不以已归档旧前端仓库推断当前可用性。脚本原件和摘要留在本地忽略目录。

采集器只允许上述 HTTPS GET 路径，不携带 Cookie 或 Authorization，不读取评论、作者资料接口。每次请求至少间隔 0.5 秒，单线程；429 和短暂服务错误有限退避，401/403立即停止该项。缓存支持重新运行及继续分页；`--refresh`重新读取时另存来源版本，不覆盖已存原文。列表只存元数据；正文只有显式公开且已发布时才入库。IP、作者资料和无关联系方式字段不存储。

## 本轮真实规模

采集标准客户端，分山劲、铁骨衣各第 1 页、每页 4 条，另补充两条已实际观察到的暗影千机来源。列表含置顶项，因此本轮是小批路径验收，不是完整覆盖估计。

| 指标 | 结果 |
| --- | ---: |
| 尝试来源 | 10 |
| 取得明确公开正文 | 9 |
| 独立宏块 | 40 |
| 分山劲 / 铁骨衣宏块 | 20 / 20 |
| 来源家族 | 8 |
| 保守结构解析通过 | 39 |
| 不支持但保留原文 | 1 |
| 不同原文哈希 / 有效结构哈希 | 40 / 39 |
| 暗影千机 / 苍生铸世 / 丝路风语宏块 | 27 / 10 / 3 |
| 已完成原生合同认证案例 | 1 |
| 已训练宏库模型 | 0 |

一个列表项的正文响应没有同时满足明确公开与已发布检查，未保存其正文。一个苍生铸世宏块使用按技能名写出的 `skill:` 条件，而当前项目解析器该条件要求数字 ID；记录为不支持，未改写或删除该行。赛季标签不能证明是正式服或测试服，`test_server`仍为未知。旧赛季及苍生铸世来源没有被自动映射到当前测试服实现。

## 清洗与 schema

新增离线入口为 `tools/exact_macro_corpus.py`，无额外 Python 包依赖。`schema`导出 `exact-macro-corpus-v1` 的 JSON Schema。每个来源宏块各占一条记录，保存来源 URL/ID、公开作者署名和 ID、发布时间/更新时间/采集时间、客户端、原始赛季标签、心法、分页请求、原文、描述与手动操作证据、未知配置及来源关系。

边界如下：

- 原文按 API 返回的 Unicode 文本保存，包括原始换行与阈值精度；文本 SHA-256与规范化结构摘要分别保存。规范化结果另存，不覆盖原文。
- 多个来源宏块不拼接。原生 `#page`结构与体态选择保留；缺失的切页规则、辅助宏按键及手动步骤保持未知。文章或宏块明确描述的预释放、辅助操作随来源保存。
- 条件结构按原生等优先级、右结合读取；`/cast`与`/fcast`身份独立。不支持的命令、条件或分页方式保留原行和原因，不删除后升级状态。
- `parsed_ast_hash`来自保守 Python结构读取器，schema标为`native-compatible-conservative-subset-v1`，**不是 Rust AST导出摘要**。`parse_status=parsed_subset`只说明当前结构范围可读取。`native_parse_status`与`replay_status`独立记录；原生工具真实接受后才更新前者，完整回放通过后才标记认证。
- 来源中的奇穴选择字符串、加速与装备说明保持原字段。未把网站奇穴槽位猜成项目奇穴 ID；属性、装备、秘籍、目标、团辅、阵法、预释放、延迟及初始状态缺失时分别记录，不伪造原作者配置。
- 本地 schema、manifest、原始版本、轨迹和完整结果都位于`backend/target/exact-macro-corpus/`，已核对为 Git忽略范围。工具拒绝将语料输出到该目录以外。

## 家族划分与 B 隔离

分组先于场景轨迹和后续增强。同一来源、显式引用来源、同一公开作者、相同 AST或只改变数值常量的相同结构做传递合并。按作者合并属于保守防泄漏策略，可能把独立作品合并为更大的组；不能把家族数当作独立配置数。

武学助手来源标记在来源标题、宏块及全文投影中检查。外部隔离清单可按来源 ID、公开作者 ID、文本/结构摘要指定固定 B及其同源关系；隔离向整个家族传播并持久化。不能以“未命中标记”直接证明无关系，未核实来源默认`review`。当前固定325步分山武学助手 B的正文、变体及轨迹未读取或使用。

本轮对已认证铁骨组进行了来源证据审核：[101545](https://www.jx3box.com/macro/101545)明确链接[106800](https://www.jx3box.com/macro/106800)，二者一起划为同源组。两份已采集来源均为铁骨，完整来源投影未见武学助手标记；该整组保留作明确测试组，未加入训练。来源审核由本次已授权工作完成，是证据分类，不是新增用户权限确认。

| 划分 | 家族 | 来源 | 宏块 |
| --- | ---: | ---: | ---: |
| `review` | 6 | 6 | 26 |
| `test` | 1 | 2 | 8 |
| `quarantine` | 1 | 1 | 6 |
| `train` / `validation` | 0 / 0 | 0 / 0 | 0 / 0 |

未执行来源审核时，默认结果为`review`34块、`quarantine`6块。明确测试组的来源版本摘要保留；正文、赛季、客户端或来源关系变化时审核失效。新加入的未核实同源成员会使整个家族回到`review`。未见测试组不用于特征选择、调参或训练；本次仅验证数据生产能力。

## D2 原生兼容案例

选择[铁骨公开来源101545](https://www.jx3box.com/macro/101545)的第4个独立宏块，在明确写出的**项目构造场景**中验证。使用项目已有短场景fixture的属性结构，心法设为铁骨衣、版本设为暗影千机，奇穴为15072与13422、初始怒气100、网络延迟0；其余装备、团辅、阵法、目标及预释放均在场景文件显式记录。此配置没有声称还原作者的奇穴槽位、装备、加速、延迟或多宏操作。

先按版本检查命令技能、技能条件和宏 Buff名称；未知名称、版本/客户端不匹配都不能静默进入认证。再由现有本地`--exact-macro-oracle`执行完整宏场景，生成主动轨迹。当前release可执行文件不导出较新的具体施放检查点字段，因此此无引导案例使用原生主动序列与已导出的相对偏移，先构造一个待验证的手动目标；没有猜测绝对时间偏移。

手动目标文件只含场景和主动目标，不含来源宏正文。随后将未改写的来源宏作为独立候选，重新运行同一原生oracle，固定终点和验收合同：

| 验收项 | 结果 |
| --- | --- |
| 版本 / 心法 | `AnYingQianJi / TieGuYi` |
| 终点 / 绝对时间容差 | 8.0秒 / 62.5ms |
| 验收口径 | `skills_and_time` |
| 主动施放次数、顺序及变体 | 8/8 |
| 最大时间误差 | 0ms |
| 完整终点回放 / 截断 | 完成 / 未截断 |
| 该案例内部状态对齐 | 8/8 |

目标重建本身是待验证假设；上述结论只来自第二次独立完整回放。候选失败、空轨迹、观察预算耗尽、截断或终点未完成不能标记认证。引导轨迹缺少原生具体施放检查点时明确返回构造限制；此纵向演示入口也不把多页来源改成单页。

场景、目标合同、来源轨迹、认证结果、合同摘要、来源正文摘要与oracle可执行文件摘要均保存在本地案例目录，来源宏和轨迹不进入公开文档。一次8秒铁骨测试不是325步案例的扩展结论，也不是原作者配置已还原、DPS保证或压缩最优性证明。

## 复现与必要验证

在仓库根目录使用现有exact-macro Python环境：

```powershell
.venv/exact-macro/Scripts/python.exe tools/exact_macro_corpus.py schema --out backend/target/exact-macro-corpus/batch-2026-10-02
.venv/exact-macro/Scripts/python.exe tools/exact_macro_corpus.py collect --out backend/target/exact-macro-corpus/batch-2026-10-02 --pages 1 --per 4 --delay 0.5 --source-id 105778 --source-id 101545
.venv/exact-macro/Scripts/python.exe tools/exact_macro_corpus.py rebuild --out backend/target/exact-macro-corpus/batch-2026-10-02 --reviewed-non-b-source 101545 --reviewed-non-b-source 106800 --test-source-family 101545
.venv/exact-macro/Scripts/python.exe tools/exact_macro_corpus.py certify --out backend/target/exact-macro-corpus/batch-2026-10-02 --record jx3box:101545:block:3 --scene backend/target/exact-macro-corpus/batch-2026-10-02/project-constructed-tiegu-scene.json
.venv/exact-macro/Scripts/python.exe -m unittest discover -s backend/tests/macro_exact -p corpus_test.py
```

原始远端内容可能更新；缓存重跑复用当次归档，`--refresh`另采新版本并重新核对。工具只允许明确版本、心法、完整模拟环境、终点、容差和验收口径的场景；缺字段即拒绝。

必要回归为20/20，通过右结合、原始精度、动作身份、多块/分页保留、未知字段、未知技能/Buff/版本、不支持语法、家族传递合并、固定B隔离、整组测试、审核版本失效、分页恢复、原文输出范围、匿名缓存与登录边界、远端身份不能选择本地存储路径、完整原生认证门控。未改Rust或前端行为，因此未重复无关全量测试。没有启动HTTP服务、读取真实userdata、修改部署或训练宏库学习模型。
