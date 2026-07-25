# 检索能力三项设计:反向查询、长句切分、未命中回退

日期:2026-07-25
状态:已批准,待实施

## 背景

fojin-cli 当前的检索能力只有一条路径:汉文整串子串匹配(FTS5 trigram)。实测暴露三个缺口:

1. **无反向查询**。`foreign_text` 就在本地库里,但没有任何入口。研究者拿到一句梵文想找汉译对应,与汉→梵是同等频次的需求。
2. **长句跨分段直接失败**。`fojin parallel "觀自在菩薩行深般若波羅蜜多時照見五蘊皆空度一切苦厄舍利子色不異空"` 返回"未找到对齐"。README 现在的处理是让用户"请拆成短句分别查"——这件事工具自己能做。
3. **未命中时没有回退**。"未找到对齐"是条死路,不告诉用户哪一部分其实是有对齐的。

## 实测依据

设计决策基于以下实测数据(本机 588 MB 数据集,908,620 行):

- `parallels` 表在查询期**只读打开**(`open_compatible_db` → `open_read_only_db`),查询期无法建索引。
- `foreign_text` 全表 `instr` 扫描:热缓存 0.35s / 冷启 5.7s(102 MB 外文文本)。
- 若给 `foreign_text` 建 trigram FTS,索引约 300–400 MB,库会从 588 MB 涨到接近 1 GB。
- 长输入并非总是失败:「是故空中無色,無受想行識,無眼耳鼻舌身意」是整串直接命中的,所以不能无条件先切分。

## 已确定的决策

| 决策点 | 结论 | 理由 |
| --- | --- | --- |
| 反查索引策略 | **不建索引,全量扫描**(Rust 侧流式过滤,见组件契约) | 热 0.35–0.40s 可接受;零架构改动,不动已发布的 data-v1,老用户无需重下 183 MB。真嫌慢再加索引。 |
| 反查 CLI 形态 | **`parallel --from <lang>`** | 复用 `--lang`/`--top`/`--limit`/`--json`;命中外文行后仍按汉文分段分组,`MatchGroup` 与渲染层零改动。 |
| 切分触发时机 | **零命中时自动兜底**,`--no-split` 关闭 | 保住"整串正好命中一条长分段"的最优结果,同时修掉 README 里"请自己拆句"的痛点。 |
| 回退层次 | **最长可命中子串** | 复用现有 FTS,零额外索引,输出确定可复现,与项目定位一致。trigram 近似排序留待后续。 |
| JSON 契约 | **顶层不变 + 新增可选字段 + `schema_version`** | 旧 agent 读 `groups` 零修改继续工作。 |
| 代码组织 | **抽出 `src/search/` 编排层,按功能切文件** | 唯一能让三个 agent 真并行而不冲突的结构;切分与回退本就是纯函数,单测不需要数据库。 |

## 架构

```
src/search/mod.rs      策略编排:整串 → 切句 → 回退,产出 SearchOutcome
src/search/split.rs    纯函数:原始输入 → Vec<Segment>(按句读切,保留原文)
src/search/fallback.rs 纯函数:查询串 + 命中判定闭包 → 最长可命中子串
src/query.rs           新增 search_foreign();其余不动
src/cli.rs             新增 --from / --no-split;编排下沉到 search
src/render.rs          新增分句小节与回退提示的渲染
```

### 核心类型(三个 agent 的共享契约)

```rust
pub struct SearchOutcome {
    pub groups: Vec<MatchGroup>,              // 合并去重后的全部命中 → 顶层 JSON 契约
    pub total: usize,
    pub segments: Option<Vec<SegmentResult>>, // 仅切分发生时 Some
    pub fallback: Option<FallbackInfo>,       // 仅未切分且零命中时 Some
}

pub struct SegmentResult {
    pub text: String,                   // 原始分句(切分在归一化之前,保留原文)
    pub total: usize,                   // 该分句的命中组总数(可能大于 groups.len())
    pub groups: Vec<MatchGroup>,        // 受每段展示上限截断后的组
    pub fallback: Option<FallbackInfo>, // 该分句自己的回退
}

pub struct FallbackInfo {
    pub matched_substring: String,      // 最长可命中子串(归一化形式,见下)
    pub char_len: usize,
}
```

`FallbackInfo.matched_substring` 是**归一化形式**,不是原文形式。回退的探测在归一化串上进行,而归一化会剥离标点并折叠简繁,字符与索引都发生变化,映射回原文需要在 `normalize()` 里额外维护一张索引对照表。收益不足以抵消这份复杂度:回退子串本身就是汉字,归一化形式(简体、无标点)照样可读可复制。

`SegmentResult.text` 则是原文形式——切分发生在归一化之前,分句天然就是原文,不存在映射问题。

### 策略流水线(`search::run`,唯一编排点)

```
1. 归一化 + 长度校验(现有逻辑不动)
2. 整串查询 —— --from 指定时走 query::search_foreign,否则 query::search
3. 有命中 → 直接返回,segments/fallback 均为 None    ← 现有行为逐字节不变
4. 零命中 且 未加 --no-split 且 原文能切出 ≥2 个有效分句:
     逐句查询 → 各句结果合并去重成顶层 groups
     仍为空的分句 → 各自跑回退
     返回 segments: Some
5. 零命中 且 切不出多句:
     整串跑回退 → 返回 fallback: Some
```

两个顺序约束:

- **切分必须在归一化之前**。`normalize()` 会剥离标点,切完就没有句读边界了。
- **回退必须在切分之后**。切完仍为空的分句才值得收缩探测。

### 合并去重与 `--limit` 语义

多个分句可能命中同一组(例如相邻两句落在同一条对齐分段里)。合并规则:

- 去重键沿用现有的 `GroupKey`(`zh_text` + `cbeta_id` + `juan_num`)。
- 顺序是**按分句顺序稳定拼接后去重**,先出现者保留位置。不引入新的跨句相关度排序——那会让输出随分句数变化而漂移,违背确定性承诺。
- 顶层 `groups` 仍受 `--limit` 约束,语义与现有一致(`--all` 放开)。
- 各分句小节内部**另行**受 `min(--limit, 3)` 约束,与顶层的 `--limit` 独立计数。

### 反查的作用域限制

`--from` **不启用切分与回退**,只做整串子串匹配。切分规则按汉文句读设计,梵文用 `/` 和空格、藏文用 `།`(shad U+0F0D),要按语种各写一套规则和测试,工作量翻倍;而反查的核心价值是"有没有对应",不是长句拆解。`--from` 与 `--no-split` 同时给出时报用法错误(退出码 2)。

## 组件契约

### `search/split.rs`

- 断句字符集只取**句级标点**:`，。；：！？、` + 半角 `,.;:!?` + 换行。**不含**书名号、引号、括号、间隔号——那些出现在句中,断了会误伤。
- 切完 trim → 归一化 → 丢弃归一化后 <2 字的段。
- 有效段数 < 2 时返回"切不出多句",调用方改走整串回退路径。
- 上限 20 段。超出时只处理前 20 段,并在输出中明确告知截断了多少,不做静默丢弃。
- 返回段的**原文形式**(未归一化)用于展示。

### `search/fallback.rs`

```rust
fn longest_matching(
    query: &str,
    probe: impl Fn(&str) -> Result<bool>,
) -> Result<Option<FallbackInfo>>
```

算法用二分,依据这条单调性:**若长度 L 的某子串能命中,则它的任意长度 L-1 子串也能命中**(子串的子串仍是子串),所以"存在长度 L 的命中子串"关于 L 单调递减。对 L ∈ [2, n-1] 二分,每轮对该长度的所有起点依次探测,任一命中即该 L 可行。

- 复杂度 O(n log n) 次探测。n=20 约 60–80 次 FTS 查询 ≈ 0.1s。
- 归一化后超过 60 字直接返回 `None`,避免病态开销。
- 同长度多个起点命中时取**最靠前**的,保证确定性。
- 命中判定由闭包注入,单元测试完全不需要数据库。

### `query::search_foreign`

**匹配在 Rust 侧做,不用 SQL `instr`。** SQL 只按语种取行:

```sql
SELECT id, zh_text, zh_norm, foreign_lang, foreign_text, confidence, cbeta_id, title_zh, juan_num
FROM parallels WHERE foreign_lang = ?1
```

然后**流式**遍历结果集,在 Rust 里用 Unicode 小写折叠后做子串判定,只保留命中行。

理由是大小写与变音符号:SQLite 的 `instr` 大小写敏感,而 `LOWER()` 只处理 ASCII,折叠不了 `Ś`→`ś`。数据里句首词首字母大写很常见(例如 `Tasmāc Chāriputra ...`),用户输入 `tasmāc` 会一条都查不到。Rust 的 `to_lowercase()` 做完整 Unicode 折叠,能正确处理。

代价可控:实测读完整列 `foreign_text`(102 MB)约 0.40s 热缓存,与 SQL 侧 `instr` 扫描的 0.35s 同量级;流式遍历不会把结果集整个物化,内存有界。**实施时需实测确认**:若 Rust 侧折叠把热缓存延迟推到 2s 以上,退回 SQL `instr` 精确匹配,并在无命中时明确提示"反查区分大小写与变音符号"。

- 不做汉文归一化(简繁映射对梵藏无意义),只 trim + Unicode 小写折叠。
- 最小长度 **3 个 Unicode 字符**(trim 之后计)。IAST 下 2 字符如 `ka` 会命中上万条,无对读价值;与汉文侧的 2 字规则平行。
- 命中行按现有 `GroupKey` 分组,展示该组**全部**平行(不只命中那行),输出结构因此与正查完全一致。
- 排序复用 `group_and_rank` 骨架,把"用哪一列算贴合度"参数化:正查用 `zh_norm`,反查用 `foreign_text`。

### CLI 表面

- `--from <lang>`:值域为静态白名单(`render::lang_label` 已知的 sa/pi/bo/en/lzh/zh)。
- `--no-split`:关闭切分兜底。
- `--lang` 与 `--from` 可同时使用:`--from sa --lang bo` 表示用梵文找,只看藏文平行。

### JSON 表面

```jsonc
{
  "schema_version": 1,
  "matched": true,
  "total": 12,
  "shown": 10,
  "groups": [ /* 10 个 MatchGroup,结构与现有契约完全一致 */ ],
  "segments": [
    {"text": "觀自在菩薩行深般若波羅蜜多時", "matched": true, "total": 3,
     "groups": [ /* 最多 3 个 */ ]},
    {"text": "度一切苦厄", "matched": false, "total": 0, "groups": [],
     "fallback": {"matched_substring": "一切苦", "char_len": 3}}
  ]
}
```

(上面为便于阅读用了 jsonc 注释标注省略处;实际输出是严格 JSON,数组内容完整。)

- `segments` 仅在切分发生时出现,`fallback` 仅在未切分且零命中时出现。
- 切分模式下 `matched` 的语义 = 顶层 `groups` 非空(任一分句有命中)。
- `matched`/`total`/`shown`/`groups` 四个字段语义不变,旧消费者零修改继续工作。

### 人类可读输出

切分时按分句分小节,**每段默认最多显示 `min(--limit, 3)` 组**——5 个分句 × 默认 limit 10 = 50 组会直接刷屏,3 组是能看清的上限,`--all` 放开。

## 错误处理

| 情况 | 行为 | 退出码 |
| --- | --- | --- |
| `--from sk`(未知语种) | 参数解析期拒绝,列出可用语种,不开库 | 2 |
| `--from pi`(已知但本数据集无行) | 正常查询,如实答"当前数据集无该语种对齐" | 0 |
| `--from` 查询 < 3 字符 | 报错,与汉文侧 2 字规则平行 | 1 |
| `--from` 与 `--no-split` 同给 | 用法错误(反查本就不切分) | 2 |
| 分句超过 20 段 | 只处理前 20 段,明确告知截断了多少,非错误 | 0 |
| 回退探测期数据库出错 | 向上传播,不吞错 | 1 |
| 零命中(含回退给出建议) | 仍是成功 | 0 |

语种校验用静态白名单而非 `SELECT DISTINCT foreign_lang`——后者无索引会全表扫 908k 行。

**唯一的顺手扩围**:同一个校验器挂到 `--lang` 上,修掉现有的 `--lang sk` 静默返回"未找到对齐"的问题。不修的话 `--from` 与 `--lang` 两个语种参数行为不一致,更让人困惑。

## 测试策略

先写测试(TDD)。

- **纯单元、不碰 DB**
  - `split.rs`:表驱动——各类标点、换行、连续标点、纯标点输入、<2 字段丢弃、20 段上限。
  - `fallback.rs`:注入假探测闭包——单调性、边界、60 字上限、同长度取最靠前、无解返回 `None`。
- **内存 fixture 集成测试**(沿用 `tests/` 现有 `init_schema` 模式)
  - `search_foreign` 的语种过滤、最小长度、分组、排序。
  - `search::run` 三条路径(命中 / 切分 / 回退)。
  - 渲染分句小节、每段 3 组上限、截断告知。
  - JSON 形状:`schema_version` 存在;`segments` 仅切分时出现;四个既有字段语义不变。
- **关键回归测试**:断言**有命中的查询,输出与改动前逐字节一致**。三项功能全部只在零命中时介入,这条测试是那个承诺的守卫,必须有。
- **真实数据冒烟**:改完拿本机 588 MB 库跑三个场景各一次(反查梵文、跨分段长句、必然零命中的串),确认延迟在预估范围内。

## 实施分工

先做一次**骨架提交**(串行,不可省):定下 `SearchOutcome`/`SegmentResult`/`FallbackInfo` 三个类型、`search/mod.rs` 的编排空壳、`fallback::longest_matching` 的函数签名,并把 `--from` 与 `--no-split` 两个 flag 都先声明好(暂时不生效)。这样三个 agent 谁都不用改 `cli.rs` 的参数结构体,冲突面归零。

| agent | 独占文件 | 与他人的耦合 |
| --- | --- | --- |
| 1 · 反向查询 | `query.rs`、语种校验器 | 无 |
| 2 · 长句切分 | `search/split.rs`、`search/mod.rs`、`render.rs` | 只按骨架签名调用 agent 3 的函数 |
| 3 · 回退 | `search/fallback.rs` | 无(纯函数 + 注入闭包) |

三个 agent 各自在独立 git worktree 内工作,合并顺序 1 → 3 → 2(2 依赖 3 的实现落地才能端到端验证)。

## 明确不在本轮范围

- 给 `foreign_text` 建索引(留待反查需求验证后)。
- trigram 重叠度近似排序(回退的第二层)。
- 反查的切分与回退。
- `cite` 的 `cbeta_id` 索引、`confidence` 恒为 1.0、`texts` 按编号查等其余已识别问题——各自独立,不与本轮耦合。
