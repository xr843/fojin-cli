# cite 索引与 confidence 显示设计

日期:2026-07-26
状态:已批准,待实施

## 背景

两个独立的小改进,共同点是都在修正"实测暴露、但没人报告过"的问题。

**1. `cite` 是全表扫描。** `parallels` 表除 FTS 外没有任何索引,`cite` 的 `WHERE cbeta_id = ?1 COLLATE NOCASE` 因此扫描全部 908,620 行(588 MB)。

**2. `confidence` 全库恒为 1.0。** 908,620 行无一例外。于是人类输出里每一行末尾的 `[MITRA 1.00]` 是纯噪音,而 `group_and_rank` 按 `max_conf` 排序、`cap_per_lang` 按置信度降序,都是在做无区分度的比较。

## 实测依据

| 场景 | 无索引 | 有索引 |
| --- | --- | --- |
| `cite T0251`(端到端) | 2.34 s | ~0 ms |
| `cite T0220`(端到端) | 0.19 s | ~0 ms |
| 索引构建耗时 | — | 1.60 s |
| 数据文件增量 | — | +17.2 MB |

查询计划:无索引为 `SCAN parallels` + `USE TEMP B-TREE FOR ORDER BY`;
BINARY 索引仅得到 `SCAN parallels USING INDEX`(仍非等值查找,因为比较带 `COLLATE NOCASE`);
NOCASE 索引得到 `SEARCH parallels USING INDEX idx_parallels_cbeta (cbeta_id=?)`,
带 `--juan` 时为 `(cbeta_id=? AND juan_num=?)`,且 ORDER BY 的临时 B 树消失。

`confidence` 分布:`SELECT ROUND(confidence,1), COUNT(*) FROM parallels GROUP BY 1` → 仅一行 `(1.0, 908620)`。

## 已确定的决策

| 决策点 | 结论 | 理由 |
| --- | --- | --- |
| 索引创建时机 | **安装/更新时建 + `cite` 路径缺失时懒建** | 现有用户本地已有 588 MB 数据,不应为一个 17 MB 的索引重下 183 MB。 |
| 索引 DDL 的位置 | **Rust 侧常量,不进 `schema.sql`** | 见下节两条理由。 |
| `texts` 是否一并加索引 | **不做** | 0.913→0.121 s 要付 +34 MB,性价比远不如 cite;且 `texts` 真正的病因是把全部分组取回 Rust 再按归一化标题过滤,预聚合小表才是根治,那需要重新导出数据。 |
| `confidence` 处理 | **显示精度为 `1.00` 时不输出标签** | 当前数据集下噪音清零,未来有真实分数时自动显现,无需再改代码。 |
| JSON 的 `confidence` | **一字不动** | agent 契约的一部分,改动会破坏现有消费者。 |
| 排序逻辑 | **一字不动** | 现在是空转,但对未来有真实分数的数据是正确的,删掉是退步。 |

## 索引设计

### DDL

```sql
CREATE INDEX IF NOT EXISTS idx_parallels_cbeta
  ON parallels(cbeta_id COLLATE NOCASE, juan_num, id)
```

`COLLATE NOCASE` 不可省略。`cite` 的比较是 `cbeta_id = ?1 COLLATE NOCASE`,BINARY 排序规则的索引无法用于该比较的等值查找,实测只能退化成索引扫描。

列顺序 `(cbeta_id, juan_num, id)` 对应 `by_cbeta` 的 `WHERE cbeta_id = ?` [`AND juan_num = ?`] `ORDER BY juan_num, id`,使过滤与排序都由索引直接满足。

### 为什么不写进 `schema.sql`

1. **Python 导出管线直接执行该文件**(`export_parallels.py` 读 `SCHEMA_PATH` 并 `executescript`)。索引若在 schema 里,会在 908,620 行插入**之前**建立,显著拖慢导出。
2. **`schema.sql` 是兼容性校验的参照物**(`validate_compatibility` 依它检查必需的表与 FTS 声明)。索引绝不能成为兼容性要求——所有已下载的数据文件都没有它,把它列入参照物会让"缺索引"看起来像"数据不兼容"。

索引是对已下载产物的**本地优化**,不是发布 schema 的一部分。DDL 作为常量定义在 `src/data.rs`。

### 创建路径一:安装与更新

在 `install_candidate` 中,于**候选文件**上创建索引——即在 `verify_dataset_file` 通过之后、原子替换之前。这样:

- 失败时候选文件被清理,活跃数据不受影响(沿用既有的 `candidate.cleanup_with` 路径)。
- 替换是原子的,不存在"索引建到一半的活跃库"。
- 成本 1.6 s,相对 183 MB 下载可忽略。

`data update` 与 `data clean` 后的重新安装都走这条路径,因此更新后索引依然存在。

### 创建路径二:`cite` 缺失时懒建

只挂在 `cite` 上。`parallel` 与 `texts` 用不到这个索引,不应为它付检测成本。

流程:

1. 用**已经打开的只读连接**查一次 `sqlite_master`,判断索引是否存在(亚毫秒,不额外开连接)。
2. 存在则直接继续查询。
3. 缺失则尝试构建:
   a. `operation_lock::try_acquire`(新增:单次 `try_lock`,不等待、不打印)。拿不到说明有 `data update`/`clean` 在跑 —— **跳过,用全表扫描**。
   b. 向 stderr 打一行一次性提示,说明正在建立索引及大致耗时。
   c. 另开读写连接执行 DDL,完成后关闭,释放锁。
4. 继续用原只读连接查询。SQLite 会在后续语句准备时看到新索引。

`operation_lock` 现有的 `acquire` 会等待最长 20 分钟并在首次等待时向 stderr 打印提示 —— 这两者对查询路径都是错的,故新增 `try_acquire`。

### 降级是硬要求

只读文件系统、权限不足、磁盘空间不足、锁被占用、任何 SQLite 错误 —— **一律静默退回全表扫描,绝不让查询失败**。索引是优化,不是功能前提。

注意其中一种容易漏掉的失败:以读写方式打开 SQLite 需要**所在目录**可写(要创建回滚日志),而不只是数据文件本身可写。因此"文件可写但目录只读"也必须落入降级路径,不能假定文件权限足以代表可写性。

唯一允许的输出是步骤 3(b) 的一次性 stderr 提示(让用户理解那一次 1–2 秒的停顿)。构建**失败**时不打印任何东西:用户没要求建索引,失败对他没有可操作性,而查询结果完全正确。

`--json` 模式下 stdout 仍是纯 JSON —— 提示走 stderr,契约不变。

### 与既有机制的关系

- `validate_compatibility`:只检查必需的表与 FTS 声明,多一个索引不影响。
- `verify_dataset`(`PRAGMA quick_check`)与 `verify_dataset_file`(FTS integrity-check):多一个索引不影响。
- `schema.sql` 的 "write-once artifact" 注释针对的是 `UPDATE`/`DELETE` 会让 FTS 索引失步。创建索引不写 `parallels` 的任何行,不涉及该风险。
- 数据文件的 SHA-256 只在下载的压缩包上校验,安装后不再校验文件本身,故本地增建索引不破坏任何校验契约。

## confidence 设计

`render::conf_tag` 改为按**显示精度**判断:

```rust
fn conf_tag(c: Option<f64>) -> String {
    match c {
        Some(v) => {
            let shown = format!("{v:.2}");
            if shown == "1.00" {
                String::new()
            } else {
                format!("  [MITRA {shown}]")
            }
        }
        None => String::new(),
    }
}
```

按格式化后的字符串而非原始值比较,是为了避免 0.995 这类值既显示成 `[MITRA 1.00]`、又被判定为"有信息量"的自相矛盾。规则因此可以一句话说清:**标签只在它能显示出非 1.00 的数字时才出现。**

标签的**缺席**读作"无保留",语义自然,不需要额外解释。

不改动的部分:

- JSON 的 `confidence` 字段照常输出。
- `group_and_rank` 的 `max_conf` 排序键、`cap_per_lang` 的置信度降序,均保持原样。

## 测试策略

**索引**

- 单元:DDL 常量能在内存库上成功执行且幂等(连跑两次)。
- 集成(临时目录 + 真实文件):
  - 索引缺失时,`cite` 触发构建,构建后索引存在,查询结果与构建前**完全一致**。
  - 索引已存在时不重复构建、不打印提示。
  - 数据文件设为只读时,`cite` 仍返回正确结果、退出码 0、不报错(降级)。
  - 锁被占用时跳过构建、查询照常成功。
  - 安装完成后的数据带索引。`tests/data.rs` 已有走通 `ensure_data` 成功路径的测试,在其基础上追加断言即可;若该路径不便复用,则新增一条。
- 关键不变式:**索引的存在与否不改变任何查询结果**,只改变速度。需要一条测试同时对有/无索引的库跑同一 `cite` 查询并断言输出逐字节相同。

**confidence**

- `conf_tag` 的直接测试:`1.0` → 空;`0.87` → `  [MITRA 0.87]`;`0.995` → 空(显示精度为 1.00);`None` → 空。
- 渲染层:一组置信度全为 1.0 的分组,输出中不含 `[MITRA`。
- 现有测试全部使用 <1.0 的值(0.91 / 0.88 / 0.75),预期零破坏,包括 `tests/golden.rs`。

**真实数据冒烟**

- `cite T0251` 与 `cite T0220` 在索引前后的耗时与输出对比。
- `parallel "色即是空"` 确认输出中不再出现 `[MITRA 1.00]`。

## 文档

- README 第 15–16 行的首屏示例输出仍挂着 `[MITRA 1.00]`,必须更新为无标签形式。
- README 需说明:首次 `cite` 可能有一次约 1–2 秒的索引构建停顿,数据目录随之增加约 17 MB。
- CHANGELOG 在 `## [0.3.0] - Unreleased` 追加条目,措辞不得暗示已发布。

## 明确不在本轮范围

- `texts` 的索引或预聚合表。
- `data status` 展示索引状态。
- 删除或改变 `confidence` 的排序用途。
- 重新导出数据(data-v2)。
