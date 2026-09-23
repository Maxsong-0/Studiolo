"""将选定资料渲染成单个客户端的可审阅投影；不修改真实客户端配置。"""
import json
from urllib.parse import quote

from client_projection_schema import rules_source
from replica_common import encode

# 首句按规则来源区分权威，其余限制两种模式相同；vault 模式整段文字与改前逐字相同。
AUTHORITY = {"vault": "这是离线资料投影，不是权威源。权威仍是 Studiolo 供给所指向的 Vault。\n",
             "policy": "这是离线资料投影，不是权威源。规则与技能的权威是 Studiolo 策略平面的现行 effective。\n"}
LIMITS = """先检查 manifest.json 的生成时间和来源确认时间；离线无法证明资料仍然最新。
规则只是随包参考，开始工作前必须读取下列规则正文；不要只依据入口概述。
离线禁止任务认领、协调黑板写入和 A2A 协作；这些操作必须回到在线权威流程。
在线读取经过授权与审计，本地读取没有逐次审计；已导出的本地资料无法即时撤销。
投影中的原始 skills 是参考资料，不代表客户端自动发现或完整加载了这些技能。
"""


def _link(path):
    return f"[读取原文](<{quote('sources/' + path, safe='/')}>)"


def _origin(config, snapshot):
    """返回 (来源元数据, 规则条目, 技能条目, 技能说明)；条目是 (显示名, sources 下相对路径)。"""
    if rules_source(config) == "policy":
        # 策略模式只标明策略版本：Vault 副本换代不改变所选规则与技能，不写进入口文件。
        origin = (f"规则与技能来源：策略平面 effective，策略版本 `{snapshot['policy_effective']}`\n\n"
                  "导出时间、来源确认时间、读取实例、逐项策略版本与逐文件摘要见同目录 `manifest.json`。\n")
        return (origin, [(name, "policy/rules/" + name) for name in config["rules"]],
                [(name, "policy/skills/" + name + "/SKILL.md") for name in config["skills"]],
                "策略平面的 skill 是单篇文本（摘要、触发与正文），原样保存在对应的 `SKILL.md`。")
    fresh = snapshot["freshness"]
    origin = (f"来源 generation：`{fresh['generation']}`\n\n"
              f"来源摘要：`{fresh['source_digest']}`\n\n"
              "导出时间、来源确认时间、读取实例与逐文件摘要见同目录 `manifest.json`。\n")
    return (origin, [(path, path) for path in config["rules"]],
            [(path, path + "/SKILL.md") for path in config["skills"]],
            "技能的其他 Markdown 资源保留在 `sources/` 中的原相对目录；按原文引用读取。")


def render(config, snapshot):
    """源内容不改写，相对资源目录保持原样；入口时间与实例身份统一查 manifest。"""
    origin, rule_items, skill_items, skill_notice = _origin(config, snapshot)
    policy = AUTHORITY[rules_source(config)] + LIMITS
    metadata = f"项目：{config['project']}\n\n来源：{config['source_url']}\n\n" + origin
    rules = "\n".join(f"- `{label}`：{_link(path)}" for label, path in rule_items)
    skills = "\n".join(f"- `{label}`：{_link(path)}" for label, path in skill_items)
    context = ("# Studiolo 客户端资料入口\n\n" + metadata + "\n" + policy
               + "\n## 开始前读取规则\n\n" + (rules or "本配置没有选定规则。")
               + "\n\n## 按需读取技能参考\n\n" + (skills or "本配置没有选定技能。")
               + "\n\n" + skill_notice + "只导出显式选择的资料，范围外链接可能仍需在线读取。\n")
    files = {"CONTEXT.md": context.encode("utf-8")}
    for source in snapshot["sources"]:
        files["sources/" + source["path"]] = source["body"].encode("utf-8")
    wrapper = ("---\nname: studiolo-context\ndescription: 在当前项目开始工作或需要项目背景时，"
               "读取 Studiolo 本地投影并检查其来源与新鲜度限制。\n---\n\n"
               "# Studiolo 资料参考\n\n"
               "先读取相对此 SKILL.md 的 `../../../CONTEXT.md`，再读取其中列出的规则与所需技能原文。\n"
               "来源、导出时间和身份见相对此文件的 `../../../manifest.json`。\n\n" + policy)
    native = ("# Studiolo 项目接入\n\n" + metadata + "\n" + policy + "\n"
              "工具配置仅引用环境变量中的独立类型令牌，不在包中保存凭据。\n"
              "首次接入须遵循客户端项目/服务器信任提示。\n\n")
    mcp_url = config["source_url"][:-len("/read")] + "/mcp"
    if config["client"] == "claude":
        files["CLAUDE.md"] = (native + "@CONTEXT.md\n").encode("utf-8")
        files[".mcp.json"] = encode({"mcpServers": {"studiolo": {
            "type": "http", "url": mcp_url,
            "headers": {"Authorization": "Bearer ${" + config["token_env"] + "}"}}}}) + b"\n"
        skill_path = ".claude/skills/studiolo-context/SKILL.md"
    else:
        files["AGENTS.md"] = (native + "开始任何工作前，明确读取本目录的 `CONTEXT.md`，"
                              "以及其中列出的规则原文。普通链接不表示正文已经自动加载。\n").encode("utf-8")
        # JSON 基础字符串转义与本配置使用的 TOML 基础字符串相容。
        toml = ("[mcp_servers.studiolo]\nurl = " + json.dumps(mcp_url, ensure_ascii=False)
                + "\nbearer_token_env_var = " + json.dumps(config["token_env"]) + "\n")
        files[".codex/config.toml"] = toml.encode("utf-8")
        skill_path = ".agents/skills/studiolo-context/SKILL.md"
    files[skill_path] = wrapper.encode("utf-8")
    return files
