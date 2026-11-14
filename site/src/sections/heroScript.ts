/** The scripted session the hero terminal plays on a loop. */
export type Span = [text: string, cls?: string]
export interface Line {
  cmd?: string // typed after the prompt, one character at a time
  out?: Span[] // printed at once
  wait?: number // ms before the next line
}

const ok = (name: string): Line => ({ out: [[`test ${name} ... `], ['ok', 'c-green']], wait: 90 })

export const SCRIPT: Line[] = [
  { cmd: 'git log --oneline -3', wait: 250 },
  { out: [['6084971 ', 'c-yellow'], ['Release v0.2.0']] },
  { out: [['5e02992 ', 'c-yellow'], ['Reload the config when it changes']] },
  { out: [['f3199de ', 'c-yellow'], ['Split the app into modules']], wait: 500 },
  { cmd: 'cargo test', wait: 400 },
  { out: [['   Compiling', 'c-green bold'], [' nex v0.2.0']], wait: 900 },
  { out: [['running 6 tests']], wait: 200 },
  ok('config::inline_comments'),
  ok('input::app_cursor_mode'),
  ok('input::ctrl_letters'),
  ok('input::sgr_mouse'),
  ok('search::literal_matches'),
  ok('search::finds_url'),
  { out: [['test result: '], ['ok', 'c-green'], ['. 6 passed; 0 failed']], wait: 4000 },
]

export const FILES: Span[][] = [
  [['󰉋 ', 'c-blue'], ['assets', 'bold']],
  [['󰉋 ', 'c-blue'], ['docs', 'bold']],
  [['󰉋 ', 'c-blue'], ['src', 'bold']],
  [[' ', 'c-orange'], ['Cargo.toml']],
  [[' ', 'c-orange'], ['Cargo.lock']],
  [[' ', 'c-blue'], ['README.md']],
  [[' ', 'c-green'], ['install.sh']],
]
