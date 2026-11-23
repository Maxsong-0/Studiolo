// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

enum MCPCatalog {
    static let entries: [MCPCatalogEntry] = [
        MCPCatalogEntry(
            id: "github",
            name: "GitHub",
            vendor: "GitHub",
            summary: "Issues, pull requests and code across your repositories",
            category: .developer,
            asset: "mcp-github",
            colorValue: 0x24292F,
            isMonochrome: true,
            transport: .http(url: "https://api.githubcopilot.com/mcp/", headers: ["Authorization": "Bearer {token}"]),
            fields: [
                MCPField(key: "token", label: "Personal access token", placeholder: "ghp_…", kind: .secret, help: "Create a fine-grained token on the page, then copy it")
            ],
            docsURL: "https://github.com/github/github-mcp-server",
            sampleTools: ["list_issues", "create_pull_request", "search_code", "get_file_contents"],
            keysURL: "https://github.com/settings/personal-access-tokens/new"
        ),
        MCPCatalogEntry(
            id: "gitlab",
            name: "GitLab",
            vendor: "GitLab",
            summary: "Issues, merge requests, pipelines and code across your projects",
            category: .developer,
            asset: "mcp-gitlab",
            colorValue: 0xFC6D26,
            transport: .oauth(url: "https://{instance}/api/v4/mcp"),
            fields: [
                MCPField(key: "instance", label: "GitLab address", placeholder: "gitlab.com", kind: .text, help: "gitlab.com, or your own instance's host", defaultValue: "gitlab.com", isRequired: false)
            ],
            docsURL: "https://docs.gitlab.com/user/model_context_protocol/mcp_server/",
            sampleTools: ["list_merge_requests", "get_merge_request_diffs", "create_issue", "get_pipeline_jobs"]
        ),
        MCPCatalogEntry(
            id: "playwright",
            name: "Playwright",
            vendor: "Microsoft",
            summary: "Drives a real browser: open pages, click, fill forms, screenshot",
            category: .browser,
            asset: "mcp-playwright",
            colorValue: 0x2EAD33,
            transport: .stdio(command: "npx", args: ["-y", "@playwright/mcp@latest"]),
            docsURL: "https://github.com/microsoft/playwright-mcp",
            sampleTools: ["browser_navigate", "browser_click", "browser_snapshot", "browser_take_screenshot"]
        ),
        MCPCatalogEntry(
            id: "chrome-devtools",
            name: "Chrome DevTools",
            vendor: "Google",
            summary: "Inspects pages, console messages and performance traces",
            category: .browser,
            asset: "mcp-chrome-devtools",
            colorValue: 0x4285F4,
            transport: .stdio(command: "npx", args: ["-y", "chrome-devtools-mcp@latest"]),
            docsURL: "https://github.com/ChromeDevTools/chrome-devtools-mcp",
            sampleTools: ["navigate_page", "take_screenshot", "list_console_messages", "performance_start_trace"]
        ),
        MCPCatalogEntry(
            id: "context7",
            name: "Context7",
            vendor: "Upstash",
            summary: "Fresh docs and examples for the libraries you use",
            category: .knowledge,
            asset: "mcp-context7",
            colorValue: 0x5B8DEF,
            transport: .http(url: "https://mcp.context7.com/mcp", headers: ["Authorization": "Bearer {key}"]),
            fields: [
                MCPField(key: "key", label: "API key", placeholder: "ctx7sk-…", kind: .secret, help: "Optional. Raises the rate limit", isRequired: false)
            ],
            docsURL: "https://github.com/upstash/context7",
            sampleTools: ["resolve-library-id", "query-docs"],
            keysURL: "https://context7.com/dashboard"
        ),
        MCPCatalogEntry(
            id: "filesystem",
            name: "Filesystem",
            vendor: "Anthropic",
            summary: "Reads, writes and searches files in one folder",
            category: .developer,
            asset: "mcp-filesystem",
            colorValue: 0x8E8E93,
            transport: .stdio(command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "{root}"]),
            fields: [
                MCPField(key: "root", label: "Folder", placeholder: "/Users/you/Projects", kind: .path, help: "The folder the server may use", defaultValue: NSHomeDirectory())
            ],
            docsURL: "https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem",
            sampleTools: ["read_text_file", "write_file", "list_directory", "search_files"]
        ),
        MCPCatalogEntry(
            id: "fetch",
            name: "Fetch",
            vendor: "Anthropic",
            summary: "Downloads web pages as plain text to read",
            category: .browser,
            asset: "mcp-fetch",
            colorValue: 0x0A84FF,
            transport: .stdio(command: "uvx", args: ["mcp-server-fetch"]),
            docsURL: "https://github.com/modelcontextprotocol/servers/tree/main/src/fetch",
            sampleTools: ["fetch"]
        ),
        MCPCatalogEntry(
            id: "memory",
            name: "Memory",
            vendor: "Anthropic",
            summary: "Remembers people, facts and how they connect",
            category: .knowledge,
            asset: "mcp-memory",
            colorValue: 0xAF52DE,
            transport: .stdio(command: "npx", args: ["-y", "@modelcontextprotocol/server-memory"]),
            docsURL: "https://github.com/modelcontextprotocol/servers/tree/main/src/memory",
            sampleTools: ["create_entities", "create_relations", "search_nodes", "read_graph"]
        ),
        MCPCatalogEntry(
            id: "sequential-thinking",
            name: "Sequential Thinking",
            vendor: "Anthropic",
            summary: "Thinks hard problems through step by step",
            category: .knowledge,
            asset: "mcp-sequential-thinking",
            colorValue: 0x30B0C7,
            transport: .stdio(command: "npx", args: ["-y", "@modelcontextprotocol/server-sequential-thinking"]),
            docsURL: "https://github.com/modelcontextprotocol/servers/tree/main/src/sequentialthinking",
            sampleTools: ["sequentialthinking"]
        ),
        MCPCatalogEntry(
            id: "git",
            name: "Git",
            vendor: "Anthropic",
            summary: "Status, diffs, history and commits for a repo",
            category: .developer,
            asset: "mcp-git",
            colorValue: 0xF05032,
            transport: .stdio(command: "uvx", args: ["mcp-server-git"]),
            docsURL: "https://github.com/modelcontextprotocol/servers/tree/main/src/git",
            sampleTools: ["git_status", "git_diff", "git_log", "git_commit"]
        ),
        MCPCatalogEntry(
            id: "brave-search",
            name: "Brave Search",
            vendor: "Brave",
            summary: "Searches the web, local places and news",
            category: .search,
            asset: "mcp-brave-search",
            colorValue: 0xFB542B,
            transport: .stdio(command: "npx", args: ["-y", "@brave/brave-search-mcp-server"]),
            fields: [
                MCPField(key: "key", label: "API key", placeholder: "BSA…", kind: .secret, help: "Make a key on the page, then copy it")
            ],
            env: ["BRAVE_API_KEY": "{key}"],
            docsURL: "https://github.com/brave/brave-search-mcp-server",
            sampleTools: ["brave_web_search", "brave_local_search", "brave_news_search"],
            keysURL: "https://api-dashboard.search.brave.com/app/keys"
        ),
        MCPCatalogEntry(
            id: "tavily",
            name: "Tavily",
            vendor: "Tavily",
            summary: "Searches the web and pulls out page content",
            category: .search,
            asset: "mcp-tavily",
            colorValue: 0x0D9488,
            transport: .oauth(url: "https://mcp.tavily.com/mcp"),
            docsURL: "https://github.com/tavily-ai/tavily-mcp",
            sampleTools: ["tavily-search", "tavily-extract", "tavily-crawl"]
        ),
        MCPCatalogEntry(
            id: "exa",
            name: "Exa",
            vendor: "Exa",
            summary: "Searches the web and reads pages, with code-focused answers",
            category: .search,
            asset: "mcp-exa",
            colorValue: 0x1F4FFF,
            transport: .oauth(url: "https://mcp.exa.ai/mcp"),
            docsURL: "https://github.com/exa-labs/exa-mcp-server",
            sampleTools: ["web_search_exa", "web_fetch_exa", "agent_run"]
        ),
        MCPCatalogEntry(
            id: "firecrawl",
            name: "Firecrawl",
            vendor: "Firecrawl",
            summary: "Scrapes, searches and maps whole websites",
            category: .browser,
            asset: "mcp-firecrawl",
            colorValue: 0xFF6B1A,
            transport: .stdio(command: "npx", args: ["-y", "firecrawl-mcp"]),
            fields: [
                MCPField(key: "key", label: "API key", placeholder: "fc-…", kind: .secret, help: "Make a key on the page, then copy it")
            ],
            env: ["FIRECRAWL_API_KEY": "{key}"],
            docsURL: "https://github.com/firecrawl/firecrawl-mcp-server",
            sampleTools: ["firecrawl_scrape", "firecrawl_search", "firecrawl_crawl", "firecrawl_map"],
            keysURL: "https://www.firecrawl.dev/app/api-keys"
        ),
        MCPCatalogEntry(
            id: "perplexity",
            name: "Perplexity",
            vendor: "Perplexity",
            summary: "Asks the web and answers with sources",
            category: .search,
            asset: "mcp-perplexity",
            colorValue: 0x20808D,
            transport: .stdio(command: "npx", args: ["-y", "@perplexity-ai/mcp-server"]),
            fields: [
                MCPField(key: "key", label: "API key", placeholder: "pplx-…", kind: .secret, help: "Make a key on the page, then copy it")
            ],
            env: ["PERPLEXITY_API_KEY": "{key}"],
            docsURL: "https://github.com/perplexityai/modelcontextprotocol",
            sampleTools: ["perplexity_search", "perplexity_ask", "perplexity_research", "perplexity_reason"],
            keysURL: "https://www.perplexity.ai/settings/api"
        ),
        MCPCatalogEntry(
            id: "slack",
            name: "Slack",
            vendor: "Slack",
            summary: "Lists channels, reads and posts messages",
            category: .work,
            asset: "mcp-slack",
            colorValue: 0x4A154B,
            transport: .stdio(command: "npx", args: ["-y", "slack-mcp-server@latest", "--transport", "stdio"]),
            fields: [
                MCPField(key: "token", label: "Bot token", placeholder: "xoxb-…", kind: .secret, help: "Copy the Bot User OAuth Token from your app, then invite the bot to the channels it should read")
            ],
            env: ["SLACK_MCP_XOXB_TOKEN": "{token}", "SLACK_MCP_ADD_MESSAGE_TOOL": "true"],
            docsURL: "https://github.com/korotovsky/slack-mcp-server",
            sampleTools: ["channels_list", "conversations_history", "conversations_search_messages", "conversations_add_message"],
            keysURL: "https://api.slack.com/apps"
        ),
        MCPCatalogEntry(
            id: "notion",
            name: "Notion",
            vendor: "Notion",
            summary: "Finds and edits pages in your workspace",
            category: .work,
            asset: "mcp-notion",
            colorValue: 0x000000,
            isMonochrome: true,
            transport: .oauth(url: "https://mcp.notion.com/mcp"),
            docsURL: "https://developers.notion.com/docs/mcp",
            sampleTools: ["API-post-search", "API-retrieve-a-page", "API-patch-block-children"]
        ),
        MCPCatalogEntry(
            id: "linear",
            name: "Linear",
            vendor: "Linear",
            summary: "Issues and projects in your Linear teams",
            category: .work,
            asset: "mcp-linear",
            colorValue: 0x5E6AD2,
            transport: .oauth(url: "https://mcp.linear.app/mcp"),
            docsURL: "https://linear.app/docs/mcp",
            sampleTools: ["list_issues", "create_issue", "update_issue", "list_projects"]
        ),
        MCPCatalogEntry(
            id: "atlassian",
            name: "Atlassian",
            vendor: "Atlassian",
            summary: "Jira issues and Confluence pages",
            category: .work,
            asset: "mcp-atlassian",
            colorValue: 0x0052CC,
            transport: .oauth(url: "https://mcp.atlassian.com/v2/mcp"),
            docsURL: "https://developer.atlassian.com/cloud/rovo-mcp/",
            sampleTools: ["searchJiraIssuesUsingJql", "createJiraIssue", "getConfluenceContent"]
        ),
        MCPCatalogEntry(
            id: "figma",
            name: "Figma",
            vendor: "Figma",
            summary: "Design details and screenshots from files",
            category: .work,
            asset: "mcp-figma",
            colorValue: 0xF24E1E,
            transport: .oauth(url: "https://mcp.figma.com/mcp"),
            docsURL: "https://help.figma.com/hc/en-us/articles/32132100833559",
            sampleTools: ["get_design_context", "get_screenshot", "get_metadata"]
        ),
        MCPCatalogEntry(
            id: "sentry",
            name: "Sentry",
            vendor: "Sentry",
            summary: "Error reports and which projects they hit",
            category: .developer,
            asset: "mcp-sentry",
            colorValue: 0x7B61FF,
            transport: .oauth(url: "https://mcp.sentry.dev/mcp"),
            docsURL: "https://mcp.sentry.dev",
            sampleTools: ["search_issues", "get_issue_details", "find_projects"]
        ),
        MCPCatalogEntry(
            id: "vercel",
            name: "Vercel",
            vendor: "Vercel",
            summary: "Projects, deployments and their build logs",
            category: .cloud,
            asset: "mcp-vercel",
            colorValue: 0x000000,
            isMonochrome: true,
            transport: .oauth(url: "https://mcp.vercel.com"),
            docsURL: "https://vercel.com/docs/agent-resources/vercel-mcp",
            sampleTools: ["list_projects", "list_deployments", "get_deployment_build_logs"]
        ),
        MCPCatalogEntry(
            id: "cloudflare",
            name: "Cloudflare",
            vendor: "Cloudflare",
            summary: "Workers, storage buckets and databases",
            category: .cloud,
            asset: "mcp-cloudflare",
            colorValue: 0xF38020,
            transport: .oauth(url: "https://bindings.mcp.cloudflare.com/mcp"),
            docsURL: "https://github.com/cloudflare/mcp-server-cloudflare",
            sampleTools: ["workers_list", "kv_namespaces_list", "r2_buckets_list", "d1_databases_list"]
        ),
        MCPCatalogEntry(
            id: "netlify",
            name: "Netlify",
            vendor: "Netlify",
            summary: "Sites, deploys, forms and environment variables",
            category: .cloud,
            asset: "mcp-netlify",
            colorValue: 0x00C7B7,
            transport: .oauth(url: "https://netlify-mcp.netlify.app/mcp"),
            docsURL: "https://docs.netlify.com/welcome/build-with-ai/netlify-mcp-server/",
            sampleTools: ["get-projects", "deploy-site", "manage-env-vars", "get-deploy"]
        ),
        MCPCatalogEntry(
            id: "supabase",
            name: "Supabase",
            vendor: "Supabase",
            summary: "Projects, SQL queries and database changes",
            category: .data,
            asset: "mcp-supabase",
            colorValue: 0x3ECF8E,
            transport: .oauth(url: "https://mcp.supabase.com/mcp"),
            docsURL: "https://supabase.com/docs/guides/ai-tools/mcp",
            sampleTools: ["list_projects", "execute_sql", "apply_migration", "query_logs"]
        ),
        MCPCatalogEntry(
            id: "stripe",
            name: "Stripe",
            vendor: "Stripe",
            summary: "Customers, payment links and balances",
            category: .work,
            asset: "mcp-stripe",
            colorValue: 0x635BFF,
            transport: .oauth(url: "https://mcp.stripe.com"),
            docsURL: "https://docs.stripe.com/mcp",
            sampleTools: ["create_customer", "list_customers", "create_payment_link", "retrieve_balance"]
        ),
        MCPCatalogEntry(
            id: "resend",
            name: "Resend",
            vendor: "Resend",
            summary: "Sends email and manages contacts, broadcasts and domains",
            category: .work,
            asset: "mcp-resend",
            colorValue: 0x000000,
            isMonochrome: true,
            transport: .oauth(url: "https://mcp.resend.com/mcp"),
            docsURL: "https://github.com/resend/resend-mcp",
            sampleTools: ["send-email", "list-contacts", "create-broadcast", "list-domains"]
        ),
        MCPCatalogEntry(
            id: "postgres",
            name: "PostgreSQL",
            vendor: "PostgreSQL",
            summary: "Runs SQL and explores the schema of a Postgres database",
            category: .data,
            asset: "mcp-postgres",
            colorValue: 0x4169E1,
            transport: .stdio(command: "npx", args: ["-y", "@bytebase/dbhub@latest", "--transport", "stdio", "--dsn", "{url}"]),
            fields: [
                MCPField(key: "url", label: "Connection string", placeholder: "postgresql://user:pass@host:5432/db", kind: .secret, help: "The connection string; a read-only user is safest")
            ],
            docsURL: "https://github.com/bytebase/dbhub",
            sampleTools: ["execute_sql", "search_objects"]
        ),
        MCPCatalogEntry(
            id: "sqlite",
            name: "SQLite",
            vendor: "SQLite",
            summary: "Reads and writes a local database file",
            category: .data,
            asset: "mcp-sqlite",
            colorValue: 0x0F80CC,
            transport: .stdio(command: "uvx", args: ["mcp-server-sqlite", "--db-path", "{path}"]),
            fields: [
                MCPField(key: "path", label: "Database file", placeholder: "/path/to/app.db", kind: .path, help: "Created if it does not exist")
            ],
            docsURL: "https://github.com/modelcontextprotocol/servers-archived/tree/main/src/sqlite",
            sampleTools: ["read_query", "write_query", "list_tables", "describe-table"]
        ),
        MCPCatalogEntry(
            id: "mongodb",
            name: "MongoDB",
            vendor: "MongoDB",
            summary: "Finds and changes documents in your collections",
            category: .data,
            asset: "mcp-mongodb",
            colorValue: 0x47A248,
            transport: .stdio(command: "npx", args: ["-y", "mongodb-mcp-server"]),
            fields: [
                MCPField(key: "url", label: "Connection string", placeholder: "mongodb+srv://…", kind: .secret, help: "In Atlas: Connect → Drivers, then copy the string")
            ],
            env: ["MDB_MCP_CONNECTION_STRING": "{url}"],
            docsURL: "https://github.com/mongodb-js/mongodb-mcp-server",
            sampleTools: ["find", "aggregate", "list-collections", "insert-many"],
            keysURL: "https://cloud.mongodb.com"
        ),
        MCPCatalogEntry(
            id: "huggingface",
            name: "Hugging Face",
            vendor: "Hugging Face",
            summary: "Searches models, datasets and papers",
            category: .knowledge,
            asset: "mcp-huggingface",
            colorValue: 0xFFB000,
            transport: .http(url: "https://huggingface.co/mcp", headers: ["Authorization": "Bearer {token}"]),
            fields: [
                MCPField(key: "token", label: "Access token", placeholder: "hf_…", kind: .secret, help: "Optional. Raises the limits", isRequired: false)
            ],
            docsURL: "https://github.com/huggingface/hf-mcp-server",
            sampleTools: ["hub_repo_search", "hub_repo_details", "hf_fs", "hf_whoami"],
            keysURL: "https://huggingface.co/settings/tokens"
        ),
        MCPCatalogEntry(
            id: "deepwiki",
            name: "DeepWiki",
            vendor: "Cognition",
            summary: "Reads docs and answers questions about repos",
            category: .knowledge,
            asset: "mcp-deepwiki",
            colorValue: 0x1D1D1F,
            isMonochrome: true,
            transport: .http(url: "https://mcp.deepwiki.com/mcp", headers: [:]),
            docsURL: "https://docs.devin.ai/work-with-devin/deepwiki-mcp",
            sampleTools: ["read_wiki_structure", "read_wiki_contents", "ask_question"]
        ),
        MCPCatalogEntry(
            id: "aws-docs",
            name: "AWS Docs",
            vendor: "Amazon Web Services",
            summary: "Searches and reads the AWS documentation",
            category: .cloud,
            asset: "mcp-aws-docs",
            colorValue: 0xFF9900,
            transport: .stdio(command: "uvx", args: ["awslabs.aws-documentation-mcp-server@latest"]),
            env: ["FASTMCP_LOG_LEVEL": "ERROR"],
            docsURL: "https://awslabs.github.io/mcp/servers/aws-documentation-mcp-server",
            sampleTools: ["read_documentation", "search_documentation", "recommend"]
        )
    ]

    static func entry(id: String) -> MCPCatalogEntry? {
        entries.first { $0.id == id }
    }

    static func entry(matching serverName: String) -> MCPCatalogEntry? {
        func normalize(_ value: String) -> String {
            value.lowercased()
                .replacingOccurrences(of: "_", with: "-")
                .replacingOccurrences(of: " ", with: "-")
        }
        let want = normalize(serverName)
        if let hit = entries.first(where: { normalize($0.id) == want }) {
            return hit
        }
        return entries.first { normalize($0.name) == want }
    }

    static var byCategory: [(category: MCPCategory, entries: [MCPCatalogEntry])] {
        MCPCategory.allCases.compactMap { category in
            let group = entries.filter { $0.category == category }
            return group.isEmpty ? nil : (category: category, entries: group)
        }
    }

    static let ids: [String] = entries.map(\.id)
}
