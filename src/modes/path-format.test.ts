import { describe, expect, test } from 'vitest';
import { homeRelative, prettyPath } from './Organizer';

describe('homeRelative', () => {
  test('replaces the macOS home prefix with ~', () => {
    expect(homeRelative('/Users/balakumar/.claude/plugins/x')).toBe('~/.claude/plugins/x');
  });
  test('replaces the Linux home prefix with ~', () => {
    expect(homeRelative('/home/bala/.codex/config.toml')).toBe('~/.codex/config.toml');
  });
  test('leaves non-home paths untouched', () => {
    expect(homeRelative('/opt/tools/thing')).toBe('/opt/tools/thing');
  });
});

describe('prettyPath', () => {
  test('elides the middle of a deep plugin path, keeping the identifying tail', () => {
    expect(prettyPath('/Users/balakumar/.claude/plugins/cache/claude-plugins-official/claude-md-management/1.0.0'))
      .toBe('~/.claude/…/claude-md-management/1.0.0');
  });
  test('passes short paths through home-relative, no ellipsis', () => {
    expect(prettyPath('/Users/balakumar/.claude/skills/brainstorming/SKILL.md'))
      .toBe('~/.claude/skills/brainstorming/SKILL.md');
    expect(prettyPath('/Users/balakumar/.claude.json')).toBe('~/.claude.json');
  });
  test('always keeps the last two segments visible', () => {
    const out = prettyPath('/Users/balakumar/personal/ward/src/modes/deep/nested/file.ts');
    expect(out.endsWith('nested/file.ts')).toBe(true);
    expect(out.startsWith('~/personal')).toBe(true);
    expect(out).toContain('…');
  });
});
