// Files of the demo folders (Files viewer, Find, diffs), their commits and their task worktrees.
//
// Each folder has one base tree (`codeTrees[projectId]`). A worktree or Local PR folder is a root with overrides
// on top of its folder's tree (`codeRoots[root]`); `null` deletes a file. Diffs and change letters are computed
// from those overrides, so every view of a change agrees.

import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { clock } from "../daemon";
import { ORG, P, ROOT } from "./ids";

// ---------------------------------------------------------------- fixed paths and commits

/** Folders of the task worktrees and Local PRs. Sessions working there use these as `cwd`. */
export const WT = {
  checkout: `${ROOT}/.worktrees/${P.storefront}/fix-flaky-checkout-test`,
  invoices: `${ROOT}/.worktrees/${P.billing}/rate-limit-invoices`,
  refunds: `${ROOT}/.worktrees/${P.docs}/document-refund-flow`,
  prices: `${ROOT}/.worktrees/${P.storefront}/format-prices`,
} as const;

/** Commit IDs that exist in the demo repositories (agent messages may mention them; they become links). */
export const DEMO_COMMITS = {
  storefrontHead: "4f2c9e1a7b3d5f60812c4e9a0b7d3f5e1c2a9b84",
  storefrontPrev: "9a1e3c5b7d2f4a6c8e0b1d3f5a7c9e2b4d6f8a10",
  checkoutFix: "c7e2a94f1b3d5e7f9a0c2e4b6d8f1a3c5e7b9d02",
  billingHead: "2b8d4f6a1c3e5b7d9f0a2c4e6b8d1f3a5c7e9b46",
  docsHead: "e5a1c3b7d9f2a4c6e8b0d1f3a5c7e9b2d4f6a813",
  refundsDoc: "71d3f5b9a2c4e6d8f0a1b3c5e7d9f2a4b6c8e0d5",
  pricesMerged: "a3c5e7b9d1f2a4c6e8b0d2f4a6c8e1b3d5f7a926",
} as const;

// ---------------------------------------------------------------- base trees

const storefront: Record<string, string> = {
  "README.md": `# storefront

The Acme web shop: product pages, cart and checkout.

## Develop

\`\`\`sh
pnpm install
pnpm dev
pnpm test
\`\`\`
`,
  "package.json": `{
  "name": "storefront",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "test": "vitest run"
  },
  "dependencies": {
    "react": "^19.0.0",
    "react-dom": "^19.0.0"
  },
  "devDependencies": {
    "typescript": "^5.6.0",
    "vite": "^6.0.0",
    "vitest": "^2.1.0"
  }
}
`,
  "tsconfig.json": `{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "jsx": "react-jsx",
    "strict": true,
    "moduleResolution": "bundler"
  },
  "include": ["src", "tests"]
}
`,
  ".gitignore": `node_modules
dist
.env
`,
  ".github/workflows/ci.yml": `name: CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
      - run: pnpm install --frozen-lockfile
      - run: pnpm test
`,
  "public/logo.svg": `<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 96 96"><rect width="96" height="96" rx="20" fill="#0f766e"/><path d="M28 62 48 26l20 36H28z" fill="#fff"/></svg>
`,
  "src/main.ts": `import { createRoot } from "react-dom/client";
import { App } from "./App";

createRoot(document.getElementById("root")!).render(<App />);
`,
  "src/cart.ts": `import { type Money, add, zero } from "./lib/money";

export type CartLine = { sku: string; name: string; unitPrice: Money; quantity: number };
export type Cart = { id: string; lines: CartLine[] };

export function cartTotal(cart: Cart): Money {
  return cart.lines.reduce((sum, line) => add(sum, { ...line.unitPrice, cents: line.unitPrice.cents * line.quantity }), zero("USD"));
}

export function addLine(cart: Cart, line: CartLine): Cart {
  const existing = cart.lines.find((l) => l.sku === line.sku);
  if (existing) return { ...cart, lines: cart.lines.map((l) => (l.sku === line.sku ? { ...l, quantity: l.quantity + line.quantity } : l)) };
  return { ...cart, lines: [...cart.lines, line] };
}

export function removeLine(cart: Cart, sku: string): Cart {
  return { ...cart, lines: cart.lines.filter((l) => l.sku !== sku) };
}
`,
  "src/checkout.ts": `import type { Cart } from "./cart";
import { cartTotal } from "./cart";
import { createOrder, type Order } from "./api/orders";

export type CheckoutState = "idle" | "submitting" | "done" | "failed";

/** Submits the cart as an order. Retries once when the network drops. */
export async function submitCheckout(cart: Cart, email: string): Promise<Order> {
  if (!cart.lines.length) throw new Error("The cart is empty");
  const total = cartTotal(cart);
  try {
    return await createOrder({ cartId: cart.id, email, total });
  } catch (e) {
    if (e instanceof TypeError) return createOrder({ cartId: cart.id, email, total });
    throw e;
  }
}
`,
  "src/api/client.ts": `const BASE = import.meta.env.VITE_API_URL ?? "/api";

export async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const res = await fetch(\`\${BASE}\${path}\`, { ...init, headers: { "Content-Type": "application/json", ...init.headers } });
  if (!res.ok) throw new Error(\`\${res.status} \${res.statusText}\`);
  return res.json() as Promise<T>;
}
`,
  "src/api/orders.ts": `import type { Money } from "../lib/money";
import { request } from "./client";

export type Order = { id: string; status: "pending" | "paid"; total: Money };

export function createOrder(body: { cartId: string; email: string; total: Money }): Promise<Order> {
  return request<Order>("/orders", { method: "POST", body: JSON.stringify(body) });
}
`,
  "src/components/CartView.tsx": `import type { Cart } from "../cart";
import { cartTotal } from "../cart";
import { formatMoney } from "../lib/format";

export function CartView({ cart, onRemove }: { cart: Cart; onRemove: (sku: string) => void }) {
  return (
    <section>
      {cart.lines.map((line) => (
        <div key={line.sku} className="cart-line">
          <span>{line.name} × {line.quantity}</span>
          <button onClick={() => onRemove(line.sku)}>Remove</button>
        </div>
      ))}
      <strong>Total: {formatMoney(cartTotal(cart))}</strong>
    </section>
  );
}
`,
  "src/components/CheckoutForm.tsx": `import { useState } from "react";
import type { Cart } from "../cart";
import { submitCheckout, type CheckoutState } from "../checkout";

export function CheckoutForm({ cart }: { cart: Cart }) {
  const [email, setEmail] = useState("");
  const [state, setState] = useState<CheckoutState>("idle");
  async function submit() {
    setState("submitting");
    try { await submitCheckout(cart, email); setState("done"); }
    catch { setState("failed"); }
  }
  return (
    <form onSubmit={(e) => { e.preventDefault(); void submit(); }}>
      <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="Email" />
      <button disabled={state === "submitting"}>Place order</button>
      {state === "failed" && <p role="alert">The order failed. Try again.</p>}
    </form>
  );
}
`,
  "src/lib/money.ts": `export type Money = { cents: number; currency: string };

export const zero = (currency: string): Money => ({ cents: 0, currency });

export function add(a: Money, b: Money): Money {
  if (a.currency !== b.currency) throw new Error("Currency mismatch");
  return { cents: a.cents + b.cents, currency: a.currency };
}
`,
  "src/lib/format.ts": `import type { Money } from "./money";

export function formatMoney(m: Money, locale = "en-US"): string {
  return new Intl.NumberFormat(locale, { style: "currency", currency: m.currency }).format(m.cents / 100);
}
`,
  "tests/cart.test.ts": `import { describe, expect, it } from "vitest";
import { addLine, cartTotal } from "../src/cart";

describe("cart", () => {
  it("merges lines with the same SKU", () => {
    const cart = addLine({ id: "c1", lines: [] }, { sku: "mug", name: "Mug", unitPrice: { cents: 1200, currency: "USD" }, quantity: 1 });
    expect(addLine(cart, cart.lines[0]).lines[0].quantity).toBe(2);
  });
  it("adds up the total", () => {
    const cart = { id: "c2", lines: [{ sku: "cap", name: "Cap", unitPrice: { cents: 900, currency: "USD" }, quantity: 3 }] };
    expect(cartTotal(cart).cents).toBe(2700);
  });
});
`,
  "tests/checkout.test.ts": `import { describe, expect, it, vi } from "vitest";
import { submitCheckout } from "../src/checkout";

vi.mock("../src/api/orders", () => ({ createOrder: vi.fn(async () => ({ id: "o1", status: "pending", total: { cents: 1200, currency: "USD" } })) }));

describe("checkout", () => {
  it("refuses an empty cart", async () => {
    await expect(submitCheckout({ id: "c1", lines: [] }, "a@example.com")).rejects.toThrow("empty");
  });
  it("creates an order", async () => {
    const cart = { id: "c1", lines: [{ sku: "mug", name: "Mug", unitPrice: { cents: 1200, currency: "USD" }, quantity: 1 }] };
    await new Promise((r) => setTimeout(r, 300));
    expect((await submitCheckout(cart, "a@example.com")).status).toBe("pending");
  });
});
`,
};

const billing: Record<string, string> = {
  "README.md": `# billing-api

Invoices, payments and webhooks for Acme. A small Rust service.

    cargo run
    cargo test
`,
  "Cargo.toml": `[package]
name = "billing-api"
version = "0.9.0"
edition = "2021"

[dependencies]
axum = "0.8"
serde = { version = "1", features = ["derive"] }
sqlx = { version = "0.8", features = ["postgres", "runtime-tokio"] }
tokio = { version = "1", features = ["full"] }
`,
  "src/main.rs": `mod db;
mod error;
mod invoices;
mod routes;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let pool = db::connect().await?;
    let app = routes::router(pool);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
`,
  "src/routes.rs": `use axum::{routing::get, Router};
use sqlx::PgPool;

use crate::invoices;

pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/invoices", get(invoices::list).post(invoices::create))
        .route("/invoices/{id}", get(invoices::get))
        .with_state(pool)
}
`,
  "src/invoices.rs": `use axum::{extract::{Path, State}, Json};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::error::ApiError;

#[derive(Serialize, sqlx::FromRow)]
pub struct Invoice {
    pub id: i64,
    pub customer: String,
    pub total_cents: i64,
    pub status: String,
}

#[derive(Deserialize)]
pub struct NewInvoice {
    pub customer: String,
    pub total_cents: i64,
}

pub async fn list(State(pool): State<PgPool>) -> Result<Json<Vec<Invoice>>, ApiError> {
    Ok(Json(sqlx::query_as("SELECT * FROM invoices ORDER BY id DESC LIMIT 100").fetch_all(&pool).await?))
}

pub async fn get(State(pool): State<PgPool>, Path(id): Path<i64>) -> Result<Json<Invoice>, ApiError> {
    Ok(Json(sqlx::query_as("SELECT * FROM invoices WHERE id = $1").bind(id).fetch_one(&pool).await?))
}

pub async fn create(State(pool): State<PgPool>, Json(body): Json<NewInvoice>) -> Result<Json<Invoice>, ApiError> {
    let row = sqlx::query_as("INSERT INTO invoices (customer, total_cents, status) VALUES ($1, $2, 'open') RETURNING *")
        .bind(body.customer)
        .bind(body.total_cents)
        .fetch_one(&pool)
        .await?;
    Ok(Json(row))
}
`,
  "src/db.rs": `use sqlx::{postgres::PgPoolOptions, PgPool};

pub async fn connect() -> anyhow::Result<PgPool> {
    let url = std::env::var("DATABASE_URL")?;
    Ok(PgPoolOptions::new().max_connections(10).connect(&url).await?)
}
`,
  "src/error.rs": `use axum::{http::StatusCode, response::{IntoResponse, Response}};

pub struct ApiError(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()).into_response()
    }
}
`,
  "src/limit.rs": `//! Request limits per client. Not wired into the router yet.

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct RateLimit {
    pub max: u32,
    pub window: Duration,
    seen: HashMap<String, (Instant, u32)>,
}

impl RateLimit {
    pub fn new(max: u32, window: Duration) -> Self {
        Self { max, window, seen: HashMap::new() }
    }
}
`,
  "tests/rate_limit.rs": `#[tokio::test]
async fn invoices_return_429_after_ten_requests() {
    let app = billing_api::test_app().await;
    for _ in 0..10 {
        assert_eq!(app.get("/invoices").await.status(), 200);
    }
    assert_eq!(app.get("/invoices").await.status(), 429);
}
`,
  "migrations/001_invoices.sql": `CREATE TABLE invoices (
    id BIGSERIAL PRIMARY KEY,
    customer TEXT NOT NULL,
    total_cents BIGINT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
`,
};

const docs: Record<string, string> = {
  "README.md": `# Acme docs

Product documentation, built with MkDocs.

- \`guide/\`: how to use the shop
- \`api/\`: the billing API reference
`,
  "mkdocs.yml": `site_name: Acme docs
nav:
  - Getting started: guide/getting-started.md
  - Checkout: guide/checkout.md
  - API:
      - Invoices: api/invoices.md
      - Errors: api/errors.md
theme:
  name: material
`,
  "CONTRIBUTING.md": `# Contributing

1. Write in short sentences.
2. Run \`mkdocs serve\` and check the page before you open a PR.
3. One topic per page.
`,
  "guide/getting-started.md": `# Getting started

Create an account, add a product to the cart and go to checkout.
You can pay by card or by invoice.
`,
  "guide/checkout.md": `# Checkout

The checkout page asks for an email address and a payment method.
After you place the order, you get a confirmation email.

## Failed payments

When a payment fails, the order stays pending for 24 hours.
`,
  "api/invoices.md": `# Invoices API

\`GET /invoices\` lists the 100 newest invoices.

\`POST /invoices\` creates an invoice:

\`\`\`json
{ "customer": "cus_123", "total_cents": 4200 }
\`\`\`
`,
  "api/errors.md": `# Errors

| Status | Meaning |
| --- | --- |
| 400 | The request is not valid |
| 404 | The invoice does not exist |
| 500 | Something failed on our side |
`,
  "api/webhooks.md": `# Webhooks

Billing sends a \`invoice.paid\` event to your endpoint when an invoice is paid.
`,
};

const dotfiles: Record<string, string> = {
  "README.md": `# dotfiles

My shell, editor and terminal configuration. Run \`./install.sh\` on a new machine.
`,
  ".zshrc": `export EDITOR=nvim
export PATH="$HOME/.local/bin:$PATH"

alias gs="git status"
alias gl="git log --oneline --graph -20"

autoload -Uz compinit && compinit
`,
  ".gitconfig": `[user]
    name = Demo User
    email = demo-user@example.com
[pull]
    rebase = true
[init]
    defaultBranch = main
`,
  "tmux.conf": `set -g mouse on
set -g history-limit 50000
set -g base-index 1
`,
  "nvim/init.lua": `vim.opt.number = true
vim.opt.relativenumber = true
vim.opt.expandtab = true
vim.opt.shiftwidth = 2

vim.keymap.set("n", "<leader>w", ":write<CR>")
`,
  "install.sh": `#!/bin/sh
set -eu
for f in .zshrc .gitconfig; do
  ln -sf "$PWD/$f" "$HOME/$f"
done
mkdir -p "$HOME/.config/nvim"
ln -sf "$PWD/nvim/init.lua" "$HOME/.config/nvim/init.lua"
`,
};

const scratch: Record<string, string> = {
  "clean.py": `import csv
import sys

rows = list(csv.DictReader(open(sys.argv[1])))
seen = set()
with open("clean.csv", "w", newline="") as out:
    w = csv.DictWriter(out, fieldnames=rows[0].keys())
    w.writeheader()
    for r in rows:
        key = r["email"].strip().lower()
        if key and key not in seen:
            seen.add(key)
            w.writerow(r)
`,
  "customers.csv": `name,email,country
Ada,ada@example.com,UK
Ben,ben@example.com,US
Ada,ADA@example.com ,UK
Chen,chen@example.com,SG
`,
  "NOTES.md": `Remove duplicate customers by email (case and spaces ignored).
`,
};

// ---------------------------------------------------------------- changes in worktrees and commits

/** One changed file: `null` before means added, `null` after means deleted. */
export type Change = { path: string; before: string | null; after: string | null };

const replace = (text: string, from: string, to: string) => {
  if (!text.includes(from)) throw new Error(`demo fixture: "${from}" not found`);
  return text.replace(from, to);
};

/** The checkout test fix of the storefront task worktree (two commits ahead of main). */
const checkoutFix: Record<string, string | null> = {
  "tests/checkout.test.ts": replace(storefront["tests/checkout.test.ts"], `    await new Promise((r) => setTimeout(r, 300));
    expect((await submitCheckout(cart, "a@example.com")).status).toBe("pending");`, `    const order = await submitCheckout(cart, "a@example.com");
    expect(order.status).toBe("pending");`),
  "src/checkout.ts": replace(storefront["src/checkout.ts"], `/** Submits the cart as an order. Retries once when the network drops. */`, `/** Submits the cart as an order. Retries once when the network drops (a TypeError from fetch). */`),
};

/** The rate limit work in progress in the billing-api worktree (uncommitted). */
const invoicesWip: Record<string, string | null> = {
  "src/limit.rs": replace(billing["src/limit.rs"], `        Self { max, window, seen: HashMap::new() }
    }
}`, `        Self { max, window, seen: HashMap::new() }
    }

    /// Counts one request of \`client\`. False once it made \`max\` requests in the current window.
    pub fn allow(&mut self, client: &str) -> bool {
        let now = Instant::now();
        let entry = self.seen.entry(client.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) > self.window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= self.max
    }
}`),
};

/** The refund flow page of the docs Local PR. */
export const refundsPage = `# Refunds

You can refund an order from the order page, fully or in part.

## Full refund

1. Open the order.
2. Choose **Refund**, then **Full amount**.
3. The invoice moves to \`refunded\` and the customer gets an email.

## Partial refund

Choose **Refund**, then enter the amount. The invoice stays \`paid\`
and shows the refunded amount.

## States

| State | Meaning |
| --- | --- |
| \`pending\` | The refund was sent to the payment provider |
| \`refunded\` | The money is back on the customer's card |
| \`failed\` | The provider refused the refund; try again or contact support |
`;
const refunds: Record<string, string | null> = {
  "guide/refunds.md": refundsPage,
  "mkdocs.yml": replace(docs["mkdocs.yml"], `  - Checkout: guide/checkout.md\n`, `  - Checkout: guide/checkout.md\n  - Refunds: guide/refunds.md\n`),
};

/** The merged-locally Local PR of the storefront (prices with the store currency). */
const prices: Record<string, string | null> = {
  "src/lib/format.ts": `import type { Money } from "./money";

/** Formats an amount in the store's locale; the currency comes from the amount itself. */
export function formatMoney(m: Money, locale = navigator.language): string {
  return new Intl.NumberFormat(locale, { style: "currency", currency: m.currency }).format(m.cents / 100);
}
`,
};

export type CodeRoot = { projectId: string; branch: string; base: string; overrides: Record<string, string | null>; exists: boolean };
export type DemoCommit = { sha: string; subject: string; parent: string | null; project: string; changes: Change[] };

const changesOf = (base: Record<string, string>, overrides: Record<string, string | null>): Change[] =>
  Object.entries(overrides).map(([path, after]) => ({ path, before: base[path] ?? null, after }));

export function codeWorld() {
  const codeTrees: Record<string, Record<string, string>> = {
    [P.storefront]: { ...storefront },
    [P.billing]: { ...billing },
    [P.docs]: { ...docs },
    [P.dotfiles]: { ...dotfiles },
    [P.scratch]: { ...scratch },
  };
  const codeRoots: Record<string, CodeRoot> = {
    [WT.checkout]: { projectId: P.storefront, branch: "lp/fix-flaky-checkout-test-demo-s-c", base: "main", overrides: { ...checkoutFix }, exists: true },
    [WT.invoices]: { projectId: P.billing, branch: "lp/rate-limit-invoices-demo-s-i", base: "main", overrides: { ...invoicesWip }, exists: true },
    [WT.refunds]: { projectId: P.docs, branch: "lp/document-refund-flow", base: "main", overrides: { ...refunds }, exists: true },
    [WT.prices]: { projectId: P.storefront, branch: "lp/format-prices", base: "main", overrides: { ...prices }, exists: false },
  };
  const codeCommits: DemoCommit[] = [
    { sha: DEMO_COMMITS.pricesMerged, subject: "Format prices in the visitor's locale", parent: DEMO_COMMITS.storefrontHead, project: P.storefront, changes: changesOf(storefront, prices) },
    { sha: DEMO_COMMITS.storefrontHead, subject: "Retry the order request once on network errors", parent: DEMO_COMMITS.storefrontPrev, project: P.storefront, changes: [{ path: "src/checkout.ts", before: replace(storefront["src/checkout.ts"], `  try {
    return await createOrder({ cartId: cart.id, email, total });
  } catch (e) {
    if (e instanceof TypeError) return createOrder({ cartId: cart.id, email, total });
    throw e;
  }`, `  return createOrder({ cartId: cart.id, email, total });`), after: storefront["src/checkout.ts"] }] },
    { sha: DEMO_COMMITS.storefrontPrev, subject: "Add the cart view", parent: null, project: P.storefront, changes: [{ path: "src/components/CartView.tsx", before: null, after: storefront["src/components/CartView.tsx"] }] },
    { sha: DEMO_COMMITS.checkoutFix, subject: "Wait for the order instead of a fixed timeout", parent: DEMO_COMMITS.storefrontHead, project: P.storefront, changes: changesOf(storefront, checkoutFix) },
    { sha: DEMO_COMMITS.billingHead, subject: "Add the invoices migration", parent: null, project: P.billing, changes: [{ path: "migrations/001_invoices.sql", before: null, after: billing["migrations/001_invoices.sql"] }] },
    { sha: DEMO_COMMITS.refundsDoc, subject: "Document the refund flow", parent: DEMO_COMMITS.docsHead, project: P.docs, changes: changesOf(docs, refunds) },
    { sha: DEMO_COMMITS.docsHead, subject: "Add the errors page", parent: null, project: P.docs, changes: [{ path: "api/errors.md", before: null, after: docs["api/errors.md"] }] },
  ];
  const created = (m: number) => clock.ago(m);
  const task = (id: string, root: string, session: string, project: string, extra: Partial<WorktreeInfo["worktree"]>, info: Partial<WorktreeInfo>): WorktreeInfo => ({
    worktree: {
      id, sessions: [session], project_id: project, repo: `${ROOT}/${codeProjectName(project)}`, common_dir: `${ROOT}/${codeProjectName(project)}/.git`, root, subdir: "",
      branch: codeRoots[root].branch, base_branch: "main", base_commit: project === P.storefront ? DEMO_COMMITS.storefrontHead : project === P.billing ? DEMO_COMMITS.billingHead : DEMO_COMMITS.docsHead,
      primary: true, state: "active", merge: null, merged: null, predraft_id: null, handoff_from: null, cleanup_note: null, created_at: created(30), ...extra,
    },
    project_name: codeProjectName(project), path: root, exists: true, dirty_files: 0, ahead: 0, behind: 0, target_checkout: `${ROOT}/${codeProjectName(project)}`, ...info,
  });
  const taskWorktrees: WorktreeInfo[] = [
    task("demo-wt-checkout", WT.checkout, "demo-s-checkout", P.storefront, { created_at: created(26) }, { ahead: 2 }),
    task("demo-wt-invoices", WT.invoices, "demo-s-invoices", P.billing, { created_at: created(5) }, { dirty_files: 1 }),
    task("demo-wt-refunds", WT.refunds, "demo-s-docs", P.docs, { state: "predraft", predraft_id: "demo-pr-refunds", created_at: created(185) }, { ahead: 1 }),
  ];
  return { codeTrees, codeRoots, codeCommits, taskWorktrees };
}

/** Folder names of the demo projects (the world is not built yet while this module builds it). */
export function codeProjectName(id: string): string {
  return { [P.storefront]: "storefront", [P.billing]: "billing-api", [P.docs]: "docs", [P.dotfiles]: "dotfiles", [P.scratch]: "tmp-1028-csv-cleanup" }[id] ?? id;
}

/** `owner/repo` of a demo project, for GitHub permalinks. */
export const codeRepoOf = (id: string) => ({ [P.storefront]: `${ORG}/storefront`, [P.billing]: `${ORG}/billing-api`, [P.docs]: `${ORG}/docs` } as Record<string, string>)[id] ?? null;

// ---------------------------------------------------------------- diffs

type Op = { kind: " " | "-" | "+"; text: string; a: number; b: number };

/** Line operations from `before` to `after` (LCS; the demo files are small). */
function lineOps(before: string, after: string): Op[] {
  const a = before ? before.replace(/\n$/, "").split("\n") : [];
  const b = after ? after.replace(/\n$/, "").split("\n") : [];
  const n = a.length, m = b.length;
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
  const ops: Op[] = [];
  let i = 0, j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && a[i] === b[j]) { ops.push({ kind: " ", text: a[i], a: i + 1, b: j + 1 }); i++; j++; }
    else if (j < m && (i >= n || lcs[i][j + 1] >= lcs[i + 1][j])) { ops.push({ kind: "+", text: b[j], a: i, b: j + 1 }); j++; }
    else { ops.push({ kind: "-", text: a[i], a: i + 1, b: j }); i++; }
  }
  return ops;
}

/** A unified patch (hunks only, like GitHub's `patch` field) with its line counts. */
export function unifiedDiff(before: string | null, after: string | null): { patch: string; additions: number; deletions: number } {
  const ops = lineOps(before ?? "", after ?? "");
  const changed = ops.map((o, i) => (o.kind === " " ? -1 : i)).filter((i) => i >= 0);
  const additions = ops.filter((o) => o.kind === "+").length, deletions = ops.filter((o) => o.kind === "-").length;
  if (!changed.length) return { patch: "", additions, deletions };
  // Group changes that are at most 6 lines apart into one hunk with 3 lines of context.
  const hunks: [number, number][] = [];
  for (const i of changed) {
    const last = hunks.at(-1);
    if (last && i - last[1] <= 6) last[1] = i;
    else hunks.push([i, i]);
  }
  const out: string[] = [];
  for (const [first, last] of hunks) {
    const from = Math.max(0, first - 3), to = Math.min(ops.length - 1, last + 3);
    const slice = ops.slice(from, to + 1);
    const aLines = slice.filter((o) => o.kind !== "+").length, bLines = slice.filter((o) => o.kind !== "-").length;
    const aStart = slice.find((o) => o.kind !== "+")?.a ?? slice[0].a, bStart = slice.find((o) => o.kind !== "-")?.b ?? slice[0].b;
    out.push(`@@ -${aLines ? aStart : aStart}${aLines === 1 ? "" : `,${aLines}`} +${bLines ? bStart : bStart}${bLines === 1 ? "" : `,${bLines}`} @@`);
    for (const o of slice) out.push(`${o.kind}${o.text}`);
  }
  return { patch: out.join("\n"), additions, deletions };
}

/** Changed-line bars of a file (`LineMark`s): `add`, `mod`, or `del` after a line. */
export function lineMarks(before: string | null, after: string): { start: number; end: number; kind: string }[] {
  const ops = lineOps(before ?? "", after);
  const marks: { start: number; end: number; kind: string }[] = [];
  let i = 0;
  while (i < ops.length) {
    if (ops[i].kind === " ") { i++; continue; }
    let added = 0, removed = 0, start = 0, end = 0;
    const lineBefore = ops[i].kind === "-" ? ops[i].b : ops[i].b - 1;
    while (i < ops.length && ops[i].kind !== " ") {
      if (ops[i].kind === "+") { added++; if (!start) start = ops[i].b; end = ops[i].b; } else removed++;
      i++;
    }
    if (added) marks.push({ start, end, kind: removed ? "mod" : "add" });
    else marks.push({ start: lineBefore, end: lineBefore, kind: "del" });
  }
  return marks;
}

/** The Shiki language id and display name of a path (the subset of `code::language` the demo files need). */
export function languageOf(path: string): [string, string] | null {
  const name = path.split("/").pop()!.toLowerCase();
  if (name === ".zshrc") return ["shellscript", "Shell"];
  if (name === ".gitignore" || name === ".gitconfig") return ["ini", "Config"];
  const ext = name.includes(".") ? name.split(".").pop()! : "";
  const table: Record<string, [string, string]> = {
    rs: ["rust", "Rust"], py: ["python", "Python"], ts: ["typescript", "TypeScript"], tsx: ["tsx", "TSX"], js: ["javascript", "JavaScript"],
    json: ["json", "JSON"], toml: ["toml", "TOML"], yml: ["yaml", "YAML"], yaml: ["yaml", "YAML"], md: ["markdown", "Markdown"],
    sh: ["shellscript", "Shell"], lua: ["lua", "Lua"], sql: ["sql", "SQL"], csv: ["csv", "CSV"], svg: ["xml", "SVG"], conf: ["ini", "Config"],
  };
  return table[ext] ?? null;
}
