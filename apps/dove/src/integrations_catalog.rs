//! The MCP servers Dove offers one-click, with how each one connects.
//!
//! Remote servers that support MCP sign-in use Dove's own OAuth
//! (`mcp_oauth`); the rest take a key the user pastes. Every one ends up as a
//! `McpServer` in `integrations.json` and reaches every harness through
//! Dove's MCP server, the same as a custom server.

use std::collections::BTreeMap;

use crate::assets::icons;
use crate::pages::integrations::{McpServer, Transport};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // `OAuth` is the scheme's name
pub enum Auth {
    /// Nothing to sign in to (local servers).
    None,
    /// Browser sign-in, Dove's OAuth.
    OAuth,
    /// Browser sign-in, or a pasted key in `header` (`prefix` + key).
    OAuthOrKey { header: &'static str, prefix: &'static str, label: &'static str },
    /// A pasted key only.
    Key { header: &'static str, prefix: &'static str, label: &'static str },
    /// Works without a key; a key raises limits.
    OptionalKey { header: &'static str, prefix: &'static str, label: &'static str },
}

impl Auth {
    /// The header a pasted key goes in, if keys are accepted.
    pub fn key(self) -> Option<(&'static str, &'static str, &'static str)> {
        match self {
            Auth::OAuthOrKey { header, prefix, label }
            | Auth::Key { header, prefix, label }
            | Auth::OptionalKey { header, prefix, label } => Some((header, prefix, label)),
            Auth::None | Auth::OAuth => None,
        }
    }

    pub fn oauth(self) -> bool {
        matches!(self, Auth::OAuth | Auth::OAuthOrKey { .. })
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Http(&'static str),
    Stdio(&'static str, &'static [&'static str]),
}

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub id: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    pub icon: &'static str,
    pub kind: Kind,
    pub auth: Auth,
    /// Where to get a key, or the server's docs.
    pub docs: &'static str,
    /// Agent-facing hint added to Dove's MCP instructions when connected.
    pub hint: &'static str,
}

const BEARER: &str = "Authorization";

pub const CATALOG: &[Entry] = &[
    Entry {
        id: "figma",
        name: "Figma",
        blurb: "Read frames, components, variables and code connect from your Figma files",
        icon: icons::BRAND_FIGMA,
        kind: Kind::Http("https://mcp.figma.com/mcp"),
        auth: Auth::OAuth,
        docs: "https://help.figma.com/hc/en-us/articles/32132100833559",
        hint: "figma__* tools read Figma designs; pass the frame or node URL the user shares and match the design in code.",
    },
    Entry {
        id: "github",
        name: "GitHub",
        blurb: "Issues, pull requests, code search, Actions and releases",
        icon: icons::BRAND_GITHUB,
        kind: Kind::Http("https://api.githubcopilot.com/mcp/"),
        auth: Auth::OAuthOrKey { header: BEARER, prefix: "Bearer ", label: "Personal access token" },
        docs: "https://github.com/settings/personal-access-tokens",
        hint: "",
    },
    Entry {
        id: "linear",
        name: "Linear",
        blurb: "Find, create and update issues, projects and cycles",
        icon: icons::BRAND_LINEAR,
        kind: Kind::Http("https://mcp.linear.app/mcp"),
        auth: Auth::OAuthOrKey { header: BEARER, prefix: "Bearer ", label: "API key" },
        docs: "https://linear.app/settings/account/security",
        hint: "",
    },
    Entry {
        id: "notion",
        name: "Notion",
        blurb: "Search, read and write pages and databases",
        icon: icons::BRAND_NOTION,
        kind: Kind::Http("https://mcp.notion.com/mcp"),
        auth: Auth::OAuth,
        docs: "https://developers.notion.com/docs/mcp",
        hint: "",
    },
    Entry {
        id: "sentry",
        name: "Sentry",
        blurb: "Investigate errors, traces and releases",
        icon: icons::BRAND_SENTRY,
        kind: Kind::Http("https://mcp.sentry.dev/mcp"),
        auth: Auth::OAuth,
        docs: "https://docs.sentry.io/product/sentry-mcp/",
        hint: "",
    },
    Entry {
        id: "supabase",
        name: "Supabase",
        blurb: "Tables, SQL, migrations, edge functions and logs",
        icon: icons::BRAND_SUPABASE,
        kind: Kind::Http("https://mcp.supabase.com/mcp"),
        auth: Auth::OAuthOrKey { header: BEARER, prefix: "Bearer ", label: "Personal access token" },
        docs: "https://supabase.com/dashboard/account/tokens",
        hint: "",
    },
    Entry {
        id: "stripe",
        name: "Stripe",
        blurb: "Customers, payments, products and docs search",
        icon: icons::BRAND_STRIPE,
        kind: Kind::Http("https://mcp.stripe.com"),
        auth: Auth::OAuthOrKey { header: BEARER, prefix: "Bearer ", label: "Agent API key" },
        docs: "https://docs.stripe.com/mcp",
        hint: "",
    },
    Entry {
        id: "vercel",
        name: "Vercel",
        blurb: "Projects, deployments, logs and domains",
        icon: icons::BRAND_VERCEL,
        kind: Kind::Http("https://mcp.vercel.com"),
        auth: Auth::OAuth,
        docs: "https://vercel.com/docs/mcp/vercel-mcp",
        hint: "",
    },
    Entry {
        id: "atlassian",
        name: "Atlassian",
        blurb: "Jira issues and Confluence pages",
        icon: icons::BRAND_ATLASSIAN,
        kind: Kind::Http("https://mcp.atlassian.com/v2/mcp"),
        auth: Auth::OAuth,
        docs: "https://support.atlassian.com/atlassian-rovo-mcp-server/",
        hint: "",
    },
    Entry {
        id: "cloudflare",
        name: "Cloudflare",
        blurb: "Workers, DNS, R2, D1 and the whole Cloudflare API",
        icon: icons::BRAND_CLOUDFLARE,
        kind: Kind::Http("https://mcp.cloudflare.com/mcp"),
        auth: Auth::OAuthOrKey { header: BEARER, prefix: "Bearer ", label: "API token" },
        docs: "https://dash.cloudflare.com/profile/api-tokens",
        hint: "",
    },
    Entry {
        id: "neon",
        name: "Neon",
        blurb: "Serverless Postgres: branches, queries and migrations",
        icon: icons::HARD_DRIVE,
        kind: Kind::Http("https://mcp.neon.tech/mcp"),
        auth: Auth::OAuthOrKey { header: BEARER, prefix: "Bearer ", label: "API key" },
        docs: "https://console.neon.tech/app/settings/api-keys",
        hint: "",
    },
    Entry {
        id: "posthog",
        name: "PostHog",
        blurb: "Product analytics, feature flags, experiments and errors",
        icon: icons::BRAND_POSTHOG,
        kind: Kind::Http("https://mcp.posthog.com/mcp"),
        auth: Auth::OAuth,
        docs: "https://posthog.com/docs/model-context-protocol",
        hint: "",
    },
    Entry {
        id: "context7",
        name: "Context7",
        blurb: "Up-to-date library docs and code examples for any package",
        icon: icons::DOCUMENT,
        kind: Kind::Http("https://mcp.context7.com/mcp"),
        auth: Auth::OptionalKey { header: BEARER, prefix: "Bearer ", label: "API key (optional)" },
        docs: "https://context7.com/dashboard",
        hint: "context7__* tools fetch current docs for a library; use them before writing code against an API you are unsure of.",
    },
    Entry {
        id: "exa",
        name: "Exa",
        blurb: "Web search and page contents built for agents",
        icon: icons::MAGNIFER,
        kind: Kind::Http("https://mcp.exa.ai/mcp"),
        auth: Auth::Key { header: "x-api-key", prefix: "", label: "API key" },
        docs: "https://dashboard.exa.ai/api-keys",
        hint: "",
    },
    Entry {
        id: "firecrawl",
        name: "Firecrawl",
        blurb: "Scrape, crawl and extract structured data from websites",
        icon: icons::GLOBAL,
        kind: Kind::Http("https://mcp.firecrawl.dev/v2/mcp"),
        auth: Auth::Key { header: BEARER, prefix: "Bearer ", label: "API key" },
        docs: "https://www.firecrawl.dev/app/api-keys",
        hint: "",
    },
    Entry {
        id: "playwright",
        name: "Playwright",
        blurb: "A separate headless browser agents can script (needs Node.js)",
        icon: icons::ACTION_TEST,
        kind: Kind::Stdio("npx", &["-y", "@playwright/mcp@latest", "--headless", "--isolated"]),
        auth: Auth::None,
        docs: "https://github.com/microsoft/playwright-mcp",
        hint: "",
    },
    Entry {
        id: "chrome-devtools",
        name: "Chrome DevTools",
        blurb: "Performance traces, network and console of a Chrome instance (needs Node.js)",
        icon: icons::BRAND_GOOGLECHROME,
        kind: Kind::Stdio("npx", &["-y", "chrome-devtools-mcp@latest", "--isolated"]),
        auth: Auth::None,
        docs: "https://github.com/ChromeDevTools/chrome-devtools-mcp",
        hint: "",
    },
    Entry {
        id: "supermemory",
        name: "Supermemory",
        blurb: "Long-term memory shared by every agent and harness",
        icon: icons::STAR,
        kind: Kind::Http("https://mcp.supermemory.ai/mcp"),
        auth: Auth::OAuth,
        docs: "https://supermemory.ai/docs/supermemory-mcp/introduction",
        hint: "supermemory__* tools are your long-term memory across sessions and harnesses: search_memory before \
               starting work (the project, the user's preferences, past decisions), and add_memory for durable \
               lessons, conventions and decisions. Never store secrets.",
    },
];

pub fn get(id: &str) -> Option<&'static Entry> {
    CATALOG.iter().find(|e| e.id == id)
}

impl Entry {
    /// The server config for this entry, with an optional pasted key.
    pub fn server(&self, id: String, key: Option<&str>) -> McpServer {
        let key = key.map(str::trim).filter(|k| !k.is_empty());
        let transport = match self.kind {
            Kind::Http(url) => {
                let mut headers = BTreeMap::new();
                if let (Some(key), Some((header, prefix, _))) = (key, self.auth.key()) {
                    let value = if key.starts_with(prefix) { key.to_string() } else { format!("{prefix}{key}") };
                    headers.insert(header.to_string(), value);
                }
                let oauth = self.auth.oauth() && headers.is_empty();
                Transport::Http { url: url.to_string(), headers, oauth }
            }
            Kind::Stdio(command, args) => Transport::Stdio {
                command: command.to_string(),
                args: args.iter().map(|a| a.to_string()).collect(),
                env: BTreeMap::new(),
                cwd: None,
            },
        };
        McpServer { id, name: self.name.to_string(), catalog: Some(self.id.to_string()), enabled: true, transport }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self.kind, Kind::Http(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_slugs() {
        let mut seen = std::collections::HashSet::new();
        for e in CATALOG {
            assert!(seen.insert(e.id), "duplicate {}", e.id);
            assert_eq!(crate::pages::integrations::slug(e.id), e.id);
        }
    }

    #[test]
    fn key_becomes_header_and_disables_oauth() {
        let linear = get("linear").unwrap();
        let with_key = linear.server("linear".into(), Some("lin_api_x"));
        match with_key.transport {
            Transport::Http { headers, oauth, .. } => {
                assert_eq!(headers.get("Authorization").map(String::as_str), Some("Bearer lin_api_x"));
                assert!(!oauth);
            }
            _ => panic!("remote"),
        }
        assert!(linear.server("linear".into(), None).uses_oauth());
        let exa = get("exa").unwrap().server("exa".into(), Some("k"));
        assert!(
            matches!(exa.transport, Transport::Http { ref headers, .. } if headers.get("x-api-key").map(String::as_str) == Some("k"))
        );
    }
}
