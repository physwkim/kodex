import { readFile } from 'fs';
import path from 'path';

class DataLoader {
    constructor(basePath) {
        this.basePath = basePath;
    }

    load(filename) {
        const fullPath = path.join(this.basePath, filename);
        return readFile(fullPath, 'utf-8');
    }
}

function processData(loader) {
    const data = loader.load('input.json');
    return JSON.parse(data);
}

export { DataLoader, processData };
