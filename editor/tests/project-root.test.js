import { describe, it, expect, beforeEach } from 'vitest';
import { isSupported, readFile, writeFile, listDirectory, _setRootHandleForTest } from '../project-root.js';

function createMockFileHandle(name, content) {
  return {
    getFile: async () => ({ text: async () => content }),
    createWritable: async () => {
      let buffer = content;
      return {
        write: async (data) => { buffer = data; },
        close: async () => {},
        _getContent: () => buffer,
      };
    },
  };
}

function createMockDirectoryHandle(entries = {}) {
  const dirs = {};
  const files = {};

  for (const [path, content] of Object.entries(entries)) {
    const parts = path.replace(/\\/g, '/').split('/');
    if (parts.length === 1) {
      files[parts[0]] = content;
    }
  }

  return {
    entries: async function* () {
      for (const [name] of Object.entries(files)) {
        yield [name, createMockFileHandle(name, files[name])];
      }
      for (const [name] of Object.entries(dirs)) {
        yield [name, dirs[name]];
      }
    },
    getFileHandle: async (name) => {
      if (!(name in files)) {
        throw new DOMException(`File "${name}" not found`, 'NotFoundError');
      }
      return createMockFileHandle(name, files[name]);
    },
    getDirectoryHandle: async (name) => {
      if (!(name in dirs)) {
        throw new DOMException(`Directory "${name}" not found`, 'NotFoundError');
      }
      return dirs[name];
    },
    _files: files,
    _dirs: dirs,
  };
}

describe('project-root', () => {
  beforeEach(() => {
    _setRootHandleForTest(null);
  });

  describe('isSupported', () => {
    it('returns false when FSA APIs are absent', () => {
      expect(isSupported()).toBe(false);
    });
  });

  describe('readFile', () => {
    it('reads content from a file relative to root', async () => {
      const root = createMockDirectoryHandle({ 'test.toml': 'hello world' });
      _setRootHandleForTest(root);
      const content = await readFile('test.toml');
      expect(content).toBe('hello world');
    });

    it('reads content from a nested path', async () => {
      const root = {
        getFileHandle: async (name) => {
          expect(name).toBe('default.toml');
          return createMockFileHandle('default.toml', 'nested content');
        },
        getDirectoryHandle: async (name) => {
          expect(name).toBe('worlds');
          return {
            getFileHandle: async (n) => {
              expect(n).toBe('default.toml');
              return createMockFileHandle('default.toml', 'nested content');
            },
          };
        },
      };
      _setRootHandleForTest(root);
      const content = await readFile('worlds/default.toml');
      expect(content).toBe('nested content');
    });
  });

  describe('writeFile', () => {
    it('writes content to a file that can be read back', async () => {
      let writtenContent = null;
      const root = {
        getFileHandle: async (name, opts) => {
          expect(name).toBe('output.toml');
          return {
            getFile: async () => ({ text: async () => writtenContent }),
            createWritable: async () => {
              return {
                write: async (data) => { writtenContent = data; },
                close: async () => {},
              };
            },
          };
        },
        getDirectoryHandle: async () => {
          throw new DOMException('Not found', 'NotFoundError');
        },
      };
      _setRootHandleForTest(root);

      await writeFile('output.toml', 'written content');
      const content = await readFile('output.toml');
      expect(content).toBe('written content');
    });

    it('creates intermediate directories when writing to nested path', async () => {
      let writtenContent = null;
      const createdDirs = [];

      const root = {
        getFileHandle: async (name, opts) => {
          expect(name).toBe('output.toml');
          return {
            getFile: async () => ({ text: async () => writtenContent }),
            createWritable: async () => ({
              write: async (data) => { writtenContent = data; },
              close: async () => {},
            }),
          };
        },
        getDirectoryHandle: async (name, opts) => {
          createdDirs.push(name);
          if (opts && opts.create) {
            return {
              getFileHandle: async (n, o) => root.getFileHandle(n, o),
              getDirectoryHandle: async (n, o) => root.getDirectoryHandle(n, o),
            };
          }
          throw new DOMException('Not found', 'NotFoundError');
        },
      };
      _setRootHandleForTest(root);

      await writeFile('worlds/output.toml', 'deep content');
      expect(createdDirs).toContain('worlds');
    });
  });

  describe('round trip', () => {
    it('write then read returns byte-identical content', async () => {
      const files = {};
      const root = {
        getFileHandle: async (name, opts) => {
          if (!files[name] && opts && opts.create) {
            files[name] = '';
          }
          if (!(name in files)) {
            throw new DOMException('Not found', 'NotFoundError');
          }
          const content = files[name];
          return {
            getFile: async () => ({ text: async () => content }),
            createWritable: async () => {
              return {
                write: async (data) => { files[name] = data; },
                close: async () => {},
              };
            },
          };
        },
        getDirectoryHandle: async () => { throw new DOMException('Not found', 'NotFoundError'); },
      };
      _setRootHandleForTest(root);

      const toml = '# Test World\n[global]\nseed = 42\n';
      await writeFile('test.toml', toml);
      const result = await readFile('test.toml');
      expect(result).toBe(toml);
    });
  });

  describe('listDirectory', () => {
    it('rejects when no root is selected', async () => {
      // Either throws "No project root selected" (if loadHandle resolves to
      // null) or rejects with an indexedDB error (in test env). Both are
      // "not usable" — what matters is that callers must select a root first.
      await expect(listDirectory('')).rejects.toBeDefined();
    });

    it('lists files at the root', async () => {
      const root = createMockDirectoryHandle({
        'a.toml': 'a',
        'b.toml': 'b',
      });
      _setRootHandleForTest(root);

      const entries = await listDirectory('');
      const names = entries.map((e) => e.name).sort();
      expect(names).toEqual(['a.toml', 'b.toml']);
      expect(entries.every((e) => e.kind === 'file')).toBe(true);
    });

    it('lists entries in a nested directory', async () => {
      const inner = {
        entries: async function* () {
          yield ['x.toml', { kind: 'file' }];
          yield ['y.toml', { kind: 'file' }];
        },
      };
      const root = {
        getDirectoryHandle: async (name) => {
          if (name === 'assets') {
            return {
              getDirectoryHandle: async (n) => {
                if (n === 'entities') return inner;
                throw new DOMException('Not found', 'NotFoundError');
              },
            };
          }
          throw new DOMException('Not found', 'NotFoundError');
        },
      };
      _setRootHandleForTest(root);

      const entries = await listDirectory('assets/entities');
      expect(entries.map((e) => e.name).sort()).toEqual(['x.toml', 'y.toml']);
    });

    it('distinguishes files from directories via kind', async () => {
      const root = {
        entries: async function* () {
          yield ['file.toml', { kind: 'file' }];
          yield ['subdir', { kind: 'directory' }];
        },
      };
      _setRootHandleForTest(root);

      const entries = await listDirectory('');
      const byName = Object.fromEntries(entries.map((e) => [e.name, e.kind]));
      expect(byName['file.toml']).toBe('file');
      expect(byName['subdir']).toBe('directory');
    });
  });
});
