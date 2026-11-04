--!strict
-- The engine as the screens see it (src/host.rs): shared state and calls,
-- in one place, so a fixture can stand in for the host.

export type Thread = {
	id: string,
	title: string,
	project: string,
	status: string,
	harness: string,
	updated: number,
	unread: boolean,
}

export type Change = { path: string, added: number, removed: number }

export type Tool = { name: string, state: string, output: string, changes: { Change } }

export type Entry = { id: string, kind: string, text: string, tool: Tool? }

export type Draft = { id: string, kind: string, text: string }

export type Open = { id: string, title: string, harness: string, running: boolean, status: string }

local store = {}

store.threads = ui.shared("kx.threads", {} :: { Thread })
store.projects = ui.shared("kx.projects", {} :: { { id: string, name: string, path: string } })
store.thread = ui.shared("kx.thread", nil :: Open?)
store.entries = ui.shared("kx.entries", {} :: { Entry })
store.draft = ui.shared("kx.draft", { id = "", kind = "", text = "" } :: Draft)

function store.open(id: string)
	ui.invoke("kx.open", id)
end

function store.home()
	ui.invoke("kx.new")
end

function store.interrupt()
	ui.invoke("kx.interrupt")
end

function store.send(text: string)
	return ui.call("kx.send", { text = text })
end

return store
