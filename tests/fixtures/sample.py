"""Sample module for testing extraction."""

import os
from pathlib import Path


class FileReader:
    """Reads files from disk."""

    def __init__(self, base_dir: str):
        self.base_dir = base_dir

    def read(self, filename: str) -> str:
        # NOTE: This is a simplified reader
        path = Path(self.base_dir) / filename
        return path.read_text()


class CsvParser(FileReader):
    """Parses CSV files."""

    def parse(self, filename: str) -> list:
        content = self.read(filename)
        return [line.split(",") for line in content.splitlines()]


def main():
    """Entry point."""
    reader = FileReader("/tmp")
    parser = CsvParser("/data")
    data = parser.parse("test.csv")
    print(data)
