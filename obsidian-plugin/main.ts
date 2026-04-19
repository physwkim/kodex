import { App, Modal, Notice, Plugin, PluginSettingTab, Setting, SuggestModal, TFile } from 'obsidian';
import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

interface KodexSettings {
    kodexPath: string;
    graphJsonPath: string;
}

const DEFAULT_SETTINGS: KodexSettings = {
    kodexPath: 'kodex',
    graphJsonPath: 'graph.json',
};

export default class KodexPlugin extends Plugin {
    settings: KodexSettings;

    async onload() {
        await this.loadSettings();

        // Command: Query graph
        this.addCommand({
            id: 'kodex-query',
            name: 'Query knowledge graph',
            callback: () => new QueryModal(this.app, this).open(),
        });

        // Command: Find path
        this.addCommand({
            id: 'kodex-path',
            name: 'Find path between nodes',
            callback: () => new PathModal(this.app, this).open(),
        });

        // Command: Explain node
        this.addCommand({
            id: 'kodex-explain',
            name: 'Explain current note',
            callback: () => this.explainCurrentNote(),
        });

        // Command: God nodes
        this.addCommand({
            id: 'kodex-god-nodes',
            name: 'Show god nodes (most connected)',
            callback: () => this.showGodNodes(),
        });

        // Command: Rebuild graph
        this.addCommand({
            id: 'kodex-rebuild',
            name: 'Rebuild knowledge graph',
            callback: () => this.rebuild(),
        });

        this.addSettingTab(new KodexSettingTab(this.app, this));
    }

    async loadSettings() {
        this.settings = Object.assign({}, DEFAULT_SETTINGS, await this.loadData());
    }

    async saveSettings() {
        await this.saveData(this.settings);
    }

    async runKodex(args: string): Promise<string> {
        const cmd = `${this.settings.kodexPath} ${args}`;
        try {
            const { stdout, stderr } = await execAsync(cmd, {
                cwd: this.getVaultPath(),
                timeout: 30000,
            });
            if (stderr) console.warn('kodex stderr:', stderr);
            return stdout.trim();
        } catch (e: any) {
            new Notice(`kodex error: ${e.message}`);
            throw e;
        }
    }

    getVaultPath(): string {
        return (this.app.vault.adapter as any).basePath || '.';
    }

    getGraphPath(): string {
        return this.settings.graphJsonPath;
    }

    async explainCurrentNote() {
        const file = this.app.workspace.getActiveFile();
        if (!file) {
            new Notice('No active file');
            return;
        }
        const name = file.basename;
        try {
            const result = await this.runKodex(
                `explain "${name}" --graph "${this.getGraphPath()}"`
            );
            new ResultModal(this.app, `Explain: ${name}`, result).open();
        } catch {
            // Error already shown via Notice
        }
    }

    async showGodNodes() {
        try {
            const result = await this.runKodex(
                `query "god nodes" --graph "${this.getGraphPath()}"`
            );
            new ResultModal(this.app, 'God Nodes', result).open();
        } catch {
            // Error already shown
        }
    }

    async rebuild() {
        new Notice('Rebuilding graph...');
        try {
            await this.runKodex(`update "${this.getVaultPath()}"`);
            new Notice('Graph rebuilt successfully');
        } catch {
            // Error already shown
        }
    }
}

// --- Query Modal ---
class QueryModal extends Modal {
    plugin: KodexPlugin;

    constructor(app: App, plugin: KodexPlugin) {
        super(app);
        this.plugin = plugin;
    }

    onOpen() {
        const { contentEl } = this;
        contentEl.createEl('h3', { text: 'Query Knowledge Graph' });

        const input = contentEl.createEl('input', {
            type: 'text',
            placeholder: 'e.g. how does authentication work?',
        });
        input.style.width = '100%';
        input.style.padding = '8px';
        input.style.marginBottom = '12px';

        const resultDiv = contentEl.createDiv();
        resultDiv.style.whiteSpace = 'pre-wrap';
        resultDiv.style.fontFamily = 'monospace';
        resultDiv.style.fontSize = '12px';
        resultDiv.style.maxHeight = '400px';
        resultDiv.style.overflow = 'auto';

        input.addEventListener('keydown', async (e) => {
            if (e.key === 'Enter') {
                const question = input.value.trim();
                if (!question) return;
                resultDiv.setText('Searching...');
                try {
                    const result = await this.plugin.runKodex(
                        `query "${question}" --graph "${this.plugin.getGraphPath()}"`
                    );
                    resultDiv.setText(result || 'No results found');
                } catch {
                    resultDiv.setText('Query failed');
                }
            }
        });

        input.focus();
    }

    onClose() {
        this.contentEl.empty();
    }
}

// --- Path Modal ---
class PathModal extends Modal {
    plugin: KodexPlugin;

    constructor(app: App, plugin: KodexPlugin) {
        super(app);
        this.plugin = plugin;
    }

    onOpen() {
        const { contentEl } = this;
        contentEl.createEl('h3', { text: 'Find Path Between Nodes' });

        const srcInput = contentEl.createEl('input', {
            type: 'text',
            placeholder: 'Source node (e.g. Client)',
        });
        srcInput.style.width = '100%';
        srcInput.style.padding = '8px';
        srcInput.style.marginBottom = '8px';

        const tgtInput = contentEl.createEl('input', {
            type: 'text',
            placeholder: 'Target node (e.g. Database)',
        });
        tgtInput.style.width = '100%';
        tgtInput.style.padding = '8px';
        tgtInput.style.marginBottom = '12px';

        const resultDiv = contentEl.createDiv();
        resultDiv.style.whiteSpace = 'pre-wrap';
        resultDiv.style.fontFamily = 'monospace';
        resultDiv.style.fontSize = '12px';

        tgtInput.addEventListener('keydown', async (e) => {
            if (e.key === 'Enter') {
                const src = srcInput.value.trim();
                const tgt = tgtInput.value.trim();
                if (!src || !tgt) return;
                resultDiv.setText('Searching...');
                try {
                    const result = await this.plugin.runKodex(
                        `path "${src}" "${tgt}" --graph "${this.plugin.getGraphPath()}"`
                    );
                    resultDiv.setText(result || 'No path found');
                } catch {
                    resultDiv.setText('Path finding failed');
                }
            }
        });

        srcInput.focus();
    }

    onClose() {
        this.contentEl.empty();
    }
}

// --- Result Modal ---
class ResultModal extends Modal {
    title: string;
    body: string;

    constructor(app: App, title: string, body: string) {
        super(app);
        this.title = title;
        this.body = body;
    }

    onOpen() {
        const { contentEl } = this;
        contentEl.createEl('h3', { text: this.title });
        const pre = contentEl.createEl('pre');
        pre.style.whiteSpace = 'pre-wrap';
        pre.style.fontSize = '12px';
        pre.style.maxHeight = '500px';
        pre.style.overflow = 'auto';
        pre.setText(this.body);
    }

    onClose() {
        this.contentEl.empty();
    }
}

// --- Settings ---
class KodexSettingTab extends PluginSettingTab {
    plugin: KodexPlugin;

    constructor(app: App, plugin: KodexPlugin) {
        super(app, plugin);
        this.plugin = plugin;
    }

    display(): void {
        const { containerEl } = this;
        containerEl.empty();
        containerEl.createEl('h2', { text: 'Kodex Settings' });

        new Setting(containerEl)
            .setName('kodex binary path')
            .setDesc('Path to the kodex executable')
            .addText((text) =>
                text
                    .setPlaceholder('kodex')
                    .setValue(this.plugin.settings.kodexPath)
                    .onChange(async (value) => {
                        this.plugin.settings.kodexPath = value;
                        await this.plugin.saveSettings();
                    })
            );

        new Setting(containerEl)
            .setName('graph.json path')
            .setDesc('Path to graph.json relative to vault root')
            .addText((text) =>
                text
                    .setPlaceholder('graph.json')
                    .setValue(this.plugin.settings.graphJsonPath)
                    .onChange(async (value) => {
                        this.plugin.settings.graphJsonPath = value;
                        await this.plugin.saveSettings();
                    })
            );
    }
}
