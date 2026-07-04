# 开发计划：多问题弹窗交互重构

> 关联需求：`docs/specs/multi-question-interaction-redesign.md`
> 仅前端 `src/views/PopupView.vue`（+ 可能一条 i18n）。单问题(n=1)/旧版顺序模式保持现状。计划描述方案，代码以实现为准。

## 0. 现状起点（相关部分）

- `current`：当前题指针，三源写入——`updateActiveFromScroll()`（比例阅读线）/ `onCardHover()`（hover）/
  `setActive()`（键盘、点 mousedown、textarea focus）；`activeLockUntil` + `hovering` 做多源仲裁。
- 常驻高亮：`.q-card :class="{ active: qi===current }"`；CSS `.q-card::before`（accent 底色块，`inset: 8px -12px -6px -12px`）
  + `.q-card::after`（左侧 3px accent 55% 竖条），`.q-card.active` 时 opacity 1。
- ⌘ 角标：`cardOptionHotkey(qi,i)` 仅对焦点题返回角标；`.popup.cmd-held .option .opt-sc` 高亮。
- 键盘（`onKeydown`）：`⌘↵` → `!onLastQuestion ? goNext() : submit()`；`⌘]`/`⌘[` → `goNext()/goPrev()`；
  `⌘1–9` → `toggleByIndex()`（作用 `current`）；`⌘W` → `requestCancel()`。
- 导航：`goNext/goPrev` → `goRel(±1)` → `setActive(target, true)`（夹边界、置 `current`、`markVisited`、滚入）。
- 门槛：`lastSeen`（末题 `visited` 为真）控提交出现；`visited` 由底部哨兵 IO + `setActive` 置位。
- 底部（`isMulti`）：取消 / 上一个(`⌘[`) / 下一个(`⌘↵`、`btn-primary` when `!onLastQuestion`) /
  提交(`v-if lastSeen`、`btn-primary` when `onLastQuestion`、`⌘↵` when `onLastQuestion`)。

## 1. 删除常驻高亮（F2）

- 删除左侧蓝条 `.q-card::after` + `.q-card.active::before/after` + `--q-active-tint` + 卡片 `:class="{ active }"` 绑定
  （`active` 类不再有任何 CSS，故一并删绑定）。
- **`.q-card::before` 保留但改用途**：由「常驻当前题淡底」改为「键盘跳转闪光层」——默认 `opacity:0`，仅
  `.q-card.flash::before` 渐隐（见 §2）。`.q-card > * { z-index:1 }` 保留（内容压在闪光层之上）。

## 2. 键盘焦点"闪一下"（B1–B4）

- **复用旧版当前题高亮那块区域**（用户反馈：别再改高亮区域）：保留 `.q-card::before`（`inset: 8px -12px -6px -12px`
  + 圆角 + `color-mix(accent 12%)` 淡底）+ `.q-card > * { z-index:1 }`，但由「常驻」改为默认 `opacity:0`、
  仅 `.q-card.flash::before` 跑 `@keyframes q-flash { from opacity:1 → to opacity:0 }`（0.55s ease-out）。
  `prefers-reduced-motion` 下 `.flash::before { animation:none }`（不闪）。**不再用元素级 WAAPI**。
- `flashCard(i)`：`el.classList.remove('flash'); void el.offsetWidth; el.classList.add('flash')`（强制 reflow 重放，
  B3）。卡片无 `:class` 绑定（已删 `active`），Vue 不会清掉手动加的 `.flash`。
- 触发点（**仅纯键盘**，B2）：`⌘↵`（`onCmdEnter` 跳转分支）、`⌘[`、`⌘]`（`onKeydown` 里 goNext/goPrev 后）
  调用 `flashCard(target)`；上一个/下一个**按钮**不调用（不闪）。

## 3. ⌘↵ 语义改造（3.3 / 修 bug）

- 新增纯函数（基于 `current`、`isAnswered`、`visited`、`total`）：
  - `nextUnansweredAfter(from)`：返回 `i>from` 且 `!isAnswered(i)` 的最小索引，无则 `-1`。
  - `nextUnseenAfter(from)`：返回 `i>from` 且 `!visited[i]` 的最小索引，无则 `-1`。
- 新增 `onCmdEnter()`：
  1. `u = nextUnansweredAfter(current)`；`u>=0` → `goToIdx(u)` + `flashCard(u)`，return。
  2. 否则 `lastSeen` → `submit()`，return。
  3. 否则 `s = nextUnseenAfter(current)`；`s>=0` → `goToIdx(s)` + `flashCard(s)`；兜底 `submit()`。
- **导航统一走 `goToIdx(target)`**（上一个/下一个、⌘[/⌘]、⌘↵ 共用，保证行为一致）：`setActive(i,false)` 先置当前题
  不滚动 → 若此刻焦点在某输入框则 `nextTick` 聚焦目标题输入框（触发折叠展开）→ **再 `nextTick` 后 `scrollQuestionIntoView(i)`**
  （展开稳定后滚动，修「目标题被底部 footer 挡住露不全」）；无焦点则直接滚动。`goRel(delta)=goToIdx(current+delta)`。
- `onKeydown` 的 `mod && Enter` 分支：`verticalMode` 走 `onCmdEnter()`；单问题/旧版顺序沿用现有
  `!onLastQuestion ? goNext() : submit()`（不变）。

## 4. ⌘ 按住时的焦点提示（F3）

- **无改动**：沿用现状角标（`cardOptionHotkey` 仅焦点题渲染、`.opt-sc` 平时淡显、`.popup.cmd-held .opt-sc` 变亮）。
- 不新增任何 ⌘-held 视觉（用户已确认：角标 + 键盘闪一下已足够；无选项的题按 ⌘ 本就无可选，无需额外提示）。

## 5. 底部『提交』按钮（3.5，已按验收反馈简化）

- 新增计算属性（`nextUnansweredAfter` 为 `function` 声明、已 hoist，可在 computed 内调用；其读 `isAnswered` 的
  按题数组，随作答变化自动重算）：
  - `cmdEnterWillSubmit = computed(() => verticalMode && lastSeen && nextUnansweredAfter(current) < 0)`
    （已看完 且 当前焦点之后再无未答；焦点在末题恒真）。
  - `submitShowsCmdEnter = verticalMode ? cmdEnterWillSubmit : onLastQuestion`；`submitPrimary = submitShowsCmdEnter`；
    `nextPrimary = !onLastQuestion && !submitPrimary`。
- 模板（多问题 footer）：
  - 提交：`v-if="verticalMode ? lastSeen : allViewed"`（门槛不变）；`:class="{ 'btn-primary': submitPrimary }"`；
    `⌘↵` 角标 `v-if="submitShowsCmdEnter"`；`:disabled="submitting || !canSubmit"`。
  - 下一个：`:class="{ 'btn-primary': nextPrimary }"`；角标 `v-if="!onLastQuestion"` 文案 `verticalMode ? '⌘]' : '⌘↵'`；
    `:disabled="submitting || current===total-1"`。
  - 上一个：`⌘[`（不变）、`:disabled="submitting || current===0"`。
  - **不显示「还剩 N 题」**（已删）。
- 说明：提交可见性仍由门槛 `lastSeen` 定；`⌘↵` 角标恒等于 `onCmdEnter` 的实际动作——焦点之后有未答=挂在别处(跳)、
  焦点之后无未答且看完=挂提交(提交)。「到末题即可 ⌘↵ 提交」；自由模式留空也能提交（焦点在末题 → cmdEnterWillSubmit）。

## 6. i18n

- **无新增 i18n**（「还剩 N 题」已删）。复用现有 `popup.prev/next`、`common.submit` 等。

## 7. 改动点清单（均在 `src/views/PopupView.vue`）

- **CSS**：删蓝条 `::after` + `.active` 组 + `--q-active-tint`；`::before` 改闪光层（默认透明）+ `.q-card.flash::before`
  `@keyframes q-flash` + reduced-motion；`.cmd-held` 角标样式**不动**。
- **脚本**：加 `flashCard`（class 重放）、`goToIdx`（统一导航 + 展开后滚动）、`nextUnansweredAfter`、`nextUnseenAfter`、
  `onCmdEnter`、`cmdEnterWillSubmit`、`submitShowsCmdEnter`、`submitPrimary`、`nextPrimary`；`goRel` 委托 `goToIdx`；
  `onKeydown` 的 Enter 走 `onCmdEnter`、`[`/`]` 后 `flashCard`。
- **模板**：卡片删 `:class="{active}"`；多问题 footer 提交 `btn-primary`/`⌘↵` 绑 `submitPrimary`/`submitShowsCmdEnter`、
  下一个 `nextPrimary` + 角标 `⌘]`/`⌘↵`；删「还剩 N 题」span。
- **i18n**：无（已删 `popup.unansweredRemaining`）。

## 8. 任务顺序

1. 删常驻高亮 CSS（§1），确认 hover/滚动不再跳动（§4 角标无改动）。
2. `flashCard` + 键盘跳转接入（§2），确认只键盘闪、按钮/hover 不闪。
3. `onCmdEnter` + 未答/未看查找（§3），确认不再回跳已答题、门槛与提交衔接正确。
4. 底部三阶段状态机 + i18n（§5/§6）。
5. `pnpm build`（vue-tsc）+ `./scripts/install.sh`：多问题去高亮/不跳动、双模式、`⌘↵` 全流程与三阶段、
   单问题与旧版顺序回归。

## 9. 风险与注意

- **闪光重放**：同一题连续跳需可重放——用元素级 `el.animate()` 而非切 class（B3）。
- **门槛与 `⌘↵`**：`onCmdEnter` 第 3 步用 `nextUnseenAfter` 推进门槛，保证键盘一路 `⌘↵` 能看完全部再提交；
  不要在有未看题时误提交。
- **`active` 类复用**：删高亮后 `active` 仅剩 ⌘-提示用途，勿残留旧底色规则。
- **回归**：单问题(n=1)、旧版顺序（`verticalEnabled` 关）路径的键盘/底部/焦点必须逐项不变——
  `onCmdEnter`、三阶段绑定、闪光都仅在 `verticalMode` 生效。
- **prefers-reduced-motion**：闪光降级为不闪，跳转/滚动/置焦点仍执行。
