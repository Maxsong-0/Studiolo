# Bottega 界面定稿画布的全套源（留底）

Bottega（Mac App）的界面定稿 B · Vetro 画在一张 Design 画布上。这里是那张画布发上去的全套源文件，**只用来在画布丢了的时候重建**。

- 线上画布：<https://claude.ai/artifact/XKJyeJGnhDR3gLNtkT4KuD>（私有，只有仓库主人打得开）。**以线上为准**，这里是快照。
- 快照时间：2026-11-28，线上第 3 版。`project/Main.dc.html` 的 sha256 开头 `a244d8e9`，`project/canvas.json` 开头 `c4c7e753`。
- 为什么留底：画布丢过两次（2026-11-25、2026-11-28，原因都没查清）。线上没有的时候，源只剩各会话临时目录里碰巧留着的副本，重启一次就没了。
- 决定和做法在项目档案里：ADR-047（定稿和之后每一条改动）、Bottega-5 第 7 节。

## 里面有什么

| 文件 | 是什么 |
|---|---|
| `project/Main.dc.html` | 唯一带内容的组件：整个窗口、全部样式和样例数据。改界面只改它 |
| `project/canvas.json` | 布局：27 块画板各放在哪、标题、顺序，加画布上的便签 |
| 其余 26 个 `project/*.dc.html` | 画板的壳，每个只有一行 `<dc-import name="Main" …>`，用参数指定这块画板看哪一屏、配哪张照片 |

每个文件头上的 `<script src="./support.js">` 是画布运行时，由画布类型提供，不在这里。

## 背景照片不在这里

十二张照片就是 App 用的那十二张：`bottega/Bottega/Resources/Assets.xcassets/backdrop-<名>.imageset/backdrop-<名>.jpg`。

`Main.dc.html` 里写的 `/_blob/<id>` 是**这一张线上画布**里的图片 id，换一张画布就失效：

| 照片 | 这张画布里的 id |
|---|---|
| rings | `7fb112ba8fc07577d9da0643a411b68f` |
| coast | `bdb4649f6fe15205a149171ad0bf5054` |
| shore | `87dd439b6e2e26ba9c40022ada1fb801` |
| forest | `39d452826189924a75eac6ae98d089a6` |
| aurora | `c33c034966ba662ba6ac33de8597f67b` |
| window | `82bc3a4fc704cd5f27caeb6c3b161d4a` |
| comet | `cd64767060cc92140cb5742cbde8d648` |
| sun | `dcf88cde096c389802d7cee38c88a2ab` |
| eclipse | `52cd6dde4e5d74f9554ddcdfd2b0de85` |
| meadow | `a79010028836a1ad61c72227edcfc71e` |
| hills | `28638bf231735dab0aaaeaec0a57fcfc` |
| dunes | `fe02985f099de3437124c42770fe2511` |

## 改了画布以后

画布在线上改、在线上发布。发完把改过的文件从线上取回，覆盖到 `project/` 下同名文件，更新上面的快照时间和摘要，和这次改动的代码或档案一起提交。没同步的话，这里就比线上旧：重建前先看提交日期。

## 画布丢了怎么重建

1. 先确认真丢了：读线上地址报找不到，仓库主人自己在浏览器里也打不开。重建要仓库主人点头。
2. 用 Design 画布类型新建一张空画布，标题「Bottega 前端重设计」。
3. 把十二张照片传成新画布的图片资源，记下每张的新 id。
4. 复制一份 `project/` 出来，把 `Main.dc.html` 里上表的十二个旧 id 换成新的（每个出现两处）；`canvas.json` 里 `createdOnFiles.at` 改成当时的时间。
5. 先发 `canvas.json` 和 `Main.dc.html`，再发其余 26 块画板。
6. 发完核对：线上 28 个文件、12 张图，`Main.dc.html` 和 `canvas.json` 的摘要和本地一致。
7. 把新地址写进 ADR-047（加一条，旧地址留着）、Bottega-入口、Bottega-5 开头，更新这份说明里的地址和 id 表，通知在做 Bottega 的会话。

## 在本机看

画布要登录才打得开。本机预览：把线上的 `artifact-type/dc-runtime.js` 取下来另存成 `support.js`，和这些 `.dc.html` 放在同一个临时目录，照片按 id 放到那个目录上一层的 `_blob/<id>`（不带扩展名），起一个静态文件服务器，直接开某一块画板。预览用的目录别放进仓库。
