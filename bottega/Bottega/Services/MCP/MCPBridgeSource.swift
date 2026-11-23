// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// The JavaScript bridge that exposes the connected MCP servers' tools to the
/// providers that load a file Bottega installs: Pi (an extension) and
/// Command Code (a mod). Both dynamic-import it and call `loadMCPTools()`.
///
/// Plain ESM JavaScript (Node 18+, no npm dependencies). The server list comes
/// from the file at `DROPPY_CODE_MCP_CONFIG`
/// (`{"mcpServers": { id: {command, args, env} | {type: "http", url, headers} }}`);
/// when the variable is unset or the file is missing, nothing is registered.
enum MCPBridge {
    static let mcpBridgeSource: String = #"""
    import fs from "node:fs";
    import { spawn } from "node:child_process";

    const PROTOCOL_VERSION = "2025-06-18";
    const CONNECT_TIMEOUT_MS = 15000;

    function configServers() {
      try {
        const file = process.env.DROPPY_CODE_MCP_CONFIG || "";
        if (!file) return {};
        const parsed = JSON.parse(fs.readFileSync(file, "utf8"));
        if (parsed && typeof parsed === "object" && parsed.mcpServers && typeof parsed.mcpServers === "object") return parsed.mcpServers;
      } catch (error) {
        console.error("bottega mcp: could not read config: " + errorText(error));
      }
      return {};
    }

    function errorText(error) {
      return error instanceof Error ? error.message : String(error);
    }

    function contentText(content) {
      const items = Array.isArray(content) ? content : [];
      return items.map((item) => (item && item.type === "text" && typeof item.text === "string" ? item.text : "[image]")).join("\n");
    }

    function describe(entry, fallback) {
      if (entry && typeof entry.description === "string" && entry.description) return entry.description;
      return fallback;
    }

    function schemaOf(entry) {
      if (entry && entry.inputSchema && typeof entry.inputSchema === "object") return entry.inputSchema;
      return { type: "object" };
    }

    function withTimeout(promise) {
      let timer = null;
      const timeout = new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("timed out")), CONNECT_TIMEOUT_MS);
      });
      return Promise.race([promise, timeout]).finally(() => { if (timer) clearTimeout(timer); });
    }

    function wrapTools(server, entries, callTool) {
      return entries.map((entry) => ({
        server,
        name: String(entry.name),
        description: describe(entry, String(entry.name)),
        inputSchema: schemaOf(entry),
        call: async (args) => {
          try {
            const result = await callTool(entry.name, args && typeof args === "object" ? args : {});
            if (result && result.isError) return "Error: " + (contentText(result.content) || "tool failed");
            return contentText(result ? result.content : []);
          } catch (error) {
            return "Error: " + errorText(error);
          }
        },
      }));
    }

    function connectStdio(server, entry) {
      return new Promise((resolve, reject) => {
        let child = null;
        try {
          child = spawn(entry.command, Array.isArray(entry.args) ? entry.args : [], {
            env: { ...process.env, ...((entry.env && typeof entry.env === "object") ? entry.env : {}) },
            stdio: ["pipe", "pipe", "ignore"],
          });
        } catch (error) {
          reject(error);
          return;
        }
        let id = 0;
        const pending = new Map();
        let buffer = "";
        let settled = false;
        const fail = (error) => {
          for (const resolvePending of pending.values()) resolvePending({ error: { message: errorText(error) } });
          pending.clear();
          if (settled) return;
          settled = true;
          reject(error);
        };
        child.on("error", fail);
        child.on("exit", () => fail(new Error("process exited")));
        child.stdin.on("error", () => {});
        child.stdout.on("data", (data) => {
          buffer += data.toString("utf8");
          let index = -1;
          while ((index = buffer.indexOf("\n")) >= 0) {
            const line = buffer.slice(0, index).trim();
            buffer = buffer.slice(index + 1);
            if (!line) continue;
            let message = null;
            try { message = JSON.parse(line); } catch { continue; }
            if (message && message.id !== undefined && message.id !== null && pending.has(message.id)) {
              const resolvePending = pending.get(message.id);
              pending.delete(message.id);
              resolvePending(message);
            }
          }
        });
        const send = (method, params) => new Promise((resolveSend, rejectSend) => {
          const message = { jsonrpc: "2.0", id: id++, method };
          if (params !== undefined) message.params = params;
          pending.set(message.id, resolveSend);
          child.stdin.write(JSON.stringify(message) + "\n", (error) => {
            if (error) { pending.delete(message.id); rejectSend(error); }
          });
        });
        const request = async (method, params) => {
          const response = await send(method, params);
          if (response && response.error) throw new Error(String((response.error && response.error.message) || "request failed"));
          return response ? response.result : undefined;
        };
        const notify = (method, params) => {
          const message = { jsonrpc: "2.0", method };
          if (params !== undefined) message.params = params;
          child.stdin.write(JSON.stringify(message) + "\n");
        };
        const callTool = (name, args) => request("tools/call", { name, arguments: args });
        (async () => {
          await request("initialize", { protocolVersion: PROTOCOL_VERSION, capabilities: {}, clientInfo: { name: "Bottega" } });
          notify("notifications/initialized", {});
          let cursor = undefined;
          const tools = [];
          for (;;) {
            const result = await request("tools/list", cursor === undefined ? {} : { cursor });
            for (const tool of ((result && result.tools) || [])) tools.push(tool);
            cursor = result ? result.nextCursor : undefined;
            if (!cursor) break;
          }
          settled = true;
          resolve(wrapTools(server, tools, callTool));
        })().catch(fail);
      });
    }

    async function postMessage(url, headers, payload, sessionId) {
      const requestHeaders = { ...(headers || {}), "Content-Type": "application/json", Accept: "application/json, text/event-stream" };
      if (sessionId) requestHeaders["mcp-session-id"] = sessionId;
      const response = await fetch(url, { method: "POST", headers: requestHeaders, body: JSON.stringify(payload) });
      const nextSession = response.headers.get("mcp-session-id") || sessionId || null;
      const contentType = response.headers.get("content-type") || "";
      let message = null;
      if (contentType.includes("text/event-stream")) {
        const text = await response.text();
        for (const line of text.split("\n")) {
          const trimmed = line.trim();
          if (!trimmed.startsWith("data:")) continue;
          const data = trimmed.slice("data:".length).trim();
          if (!data || data === "[DONE]") continue;
          try {
            const parsed = JSON.parse(data);
            if (parsed && typeof parsed === "object" && ("result" in parsed || "error" in parsed)) { message = parsed; break; }
          } catch {}
        }
      } else {
        const text = await response.text();
        try { message = text ? JSON.parse(text) : null; } catch { message = null; }
      }
      if (!message) throw new Error("empty response (status " + response.status + ")");
      return { message, sessionId: nextSession };
    }

    function connectRemote(server, entry) {
      return (async () => {
        const url = entry.url;
        const headers = (entry.headers && typeof entry.headers === "object") ? entry.headers : {};
        let sessionId = null;
        let id = 0;
        const request = async (method, params) => {
          const payload = { jsonrpc: "2.0", id: id++, method };
          if (params !== undefined) payload.params = params;
          const outcome = await postMessage(url, headers, payload, sessionId);
          sessionId = outcome.sessionId;
          if (outcome.message.error) throw new Error(String((outcome.message.error && outcome.message.error.message) || "request failed"));
          return outcome.message.result;
        };
        const notify = async (method, params) => {
          const payload = { jsonrpc: "2.0", method };
          if (params !== undefined) payload.params = params;
          try {
            const outcome = await postMessage(url, headers, payload, sessionId);
            sessionId = outcome.sessionId;
          } catch {}
        };
        await request("initialize", { protocolVersion: PROTOCOL_VERSION, capabilities: {}, clientInfo: { name: "Bottega" } });
        await notify("notifications/initialized", {});
        let cursor = undefined;
        const tools = [];
        for (;;) {
          const result = await request("tools/list", cursor === undefined ? {} : { cursor });
          for (const tool of ((result && result.tools) || [])) tools.push(tool);
          cursor = result ? result.nextCursor : undefined;
          if (!cursor) break;
        }
        const callTool = (name, args) => request("tools/call", { name, arguments: args });
        return wrapTools(server, tools, callTool);
      })();
    }

    export async function loadMCPTools() {
      const servers = configServers();
      const tools = [];
      for (const server of Object.keys(servers)) {
        const entry = servers[server];
        try {
          if (entry && typeof entry === "object" && entry.type === "http" && typeof entry.url === "string") {
            tools.push(...await withTimeout(connectRemote(server, entry)));
          } else if (entry && typeof entry === "object" && typeof entry.command === "string") {
            tools.push(...await withTimeout(connectStdio(server, entry)));
          }
        } catch (error) {
          console.error("bottega mcp: skipping " + server + ": " + errorText(error));
        }
      }
      return tools;
    }
    """#

    /// Writes the bridge where the Pi extension and the Command Code mod can
    /// import it, and returns its path. Rewritten only when its text changed,
    /// so a new build's bridge replaces the old one.
    static func installBridge() throws -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: LoginEnvironment.homeDirectory).appendingPathComponent("Library/Application Support")
        let directory = base.appendingPathComponent("\(AppInfo.storageName)/MCP", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let file = directory.appendingPathComponent("bottega-mcp-bridge.mjs")
        let data = Data(mcpBridgeSource.utf8)
        if (try? Data(contentsOf: file)) != data {
            try data.write(to: file, options: .atomic)
        }
        return file
    }
}
