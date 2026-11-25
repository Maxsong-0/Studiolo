看界面用的事件流，不是测试数据，也不是 LMAS 录的。

- `waiting.jsonl`：前 42 条是 LMAS 的 `fixtures/bottega/events.jsonl` 原样，后面是手编的，补上录好的那份里没有的几样：还没答的批准单（一张带看不见的字符）、问用户的问题、在做的和排队的卡、顶到上限的信、不认得的事件。
- 打开：`open --env BOTTEGA_RECORDING="$PWD/bottega/samples/waiting.jsonl" "bottega/build.noindex/dev/Build/Products/Debug/Bottega Dev.app"`，或者 Debug 版的 File > Open Recording…。录好的事件流连不到任何地方，什么都发不出去。
