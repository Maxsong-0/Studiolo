# Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import plistlib
base = '/Applications/SF Symbols.app/Contents/Resources/Metadata/'
header = plistlib.load(open(base + 'name_availability.plist', 'rb'))
avail, years = header['symbols'], header['year_to_release']
cats = plistlib.load(open(base + 'categories.plist', 'rb'))
symcats = plistlib.load(open(base + 'symbol_categories.plist', 'rb'))
pseudo = {'all', 'whatsnew', 'variable', 'multicolor'}
order = [c['key'] for c in cats if c['key'] not in pseudo]
labels = {c['key']: c['label'] for c in cats}
locale = {'ar', 'he', 'hi', 'th', 'zh', 'ja', 'ko', 'ru', 'rtl', 'ltr'}
def localised(name):
    return any(part in locale for part in name.split('.')[1:])
def on_this_macos(year):
    release = years.get(year)
    if not release:
        return False
    major, minor = release['macOS'].split('.')
    return (int(major), int(minor)) <= (26, 0)
names = [n for n in avail if n.endswith('.fill') and not localised(n) and on_this_macos(avail[n])]
names += [n for n in ['swift', 'apple.logo', 'terminal', 'curlybraces'] if n in avail]
groups = {key: [] for key in order}
other = []
for name in names:
    keys = [k for k in symcats.get(name, ()) if k in groups]
    (groups[keys[0]] if keys else other).append(name)
lines = [labels[k] + '\t' + ' '.join(sorted(groups[k])) for k in order if groups[k]]
if other:
    lines.append('Other\t' + ' '.join(sorted(other)))
body = '\n'.join(lines)
head = [
    'import Foundation',
    '',
    '/// Every SF Symbol the picker offers as a project mark, grouped under the SF Symbols app\'s',
    '/// own category names, generated from this Mac\'s SF Symbols metadata: filled marks only,',
    '/// no localised variants, and nothing newer than the system the app itself requires. `all`',
    '/// is the flat list the search reads, `groups` the sections the picker draws.',
    'enum ProjectSymbolCatalogue {',
    '    struct Group: Identifiable, Hashable {',
    '        let id: String',
    '        let title: String',
    '        let names: [String]',
    '    }',
    '',
    '    static let groups: [Group] = raw.split(separator: "\\n").compactMap { line in',
    '        let parts = line.split(separator: "\\t", maxSplits: 1, omittingEmptySubsequences: false)',
    '        guard parts.count == 2 else { return nil }',
    '        let names = parts[1].split(separator: " ").map(String.init)',
    '        guard !names.isEmpty else { return nil }',
    '        return Group(id: String(parts[0]), title: String(parts[0]), names: names)',
    '    }',
    '',
    '    static let all: [String] = groups.flatMap(\\.names)',
    '',
    '    private static let raw = """',
]
text = '\n'.join(head) + '\n' + body + '\n' + '"""' + '\n}' + '\n'
open('Bottega/UI/Chrome/ProjectSymbolCatalogue.swift', 'w').write(text)
print('names', len(names), 'groups', len(lines), 'bytes', len(text))
