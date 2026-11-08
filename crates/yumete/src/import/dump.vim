" yumete --import vim: what this Vim has once it has started, written as
" JSON to $YUMETE_DUMP. It reads and writes nothing else. maplist() and
" getscriptinfo() need Vim 9.0.
function s:Dump()
  let out = {'vim': (v:version / 100) . '.' . (v:version % 100), 'leader': get(g:, 'mapleader', v:null),
        \ 'colorscheme': get(g:, 'colors_name', v:null), 'runtime': $VIMRUNTIME,
        \ 'options': {}, 'keymaps': [], 'autocmds': [], 'loaded': []}
  if !exists('*maplist') || !exists('*getscriptinfo')
    let out.error = 'too old'
    call writefile([json_encode(out)], $YUMETE_DUMP)
    return
  endif
  let scripts = {}
  for info in getscriptinfo()
    let scripts[info.sid] = info.name
  endfor
  for name in getcompletion('', 'option')
    try
      let value = eval('&' . name)
      let w = matchlist(execute('verbose set ' . name . '?'), 'Last set from \(.\{-}\)\%( line \(\d\+\)\)\=\n')
      let out.options[name] = {'value': value, 'script': get(w, 1, v:null), 'line': str2nr(get(w, 2, '0'))}
    catch
    endtry
  endfor
  for m in maplist()
    call add(out.keymaps, {'mode': m.mode, 'lhs': m.lhs, 'rhs': m.rhs, 'expr': m.expr,
          \ 'script': get(scripts, m.sid, v:null), 'line': m.lnum})
  endfor
  for a in autocmd_get({'event': 'TextYankPost'}) + autocmd_get({'event': 'FileType'})
    call add(out.autocmds, {'event': a.event, 'pattern': a.pattern, 'group': get(a, 'group', ''), 'command': a.cmd})
  endfor
  call writefile([json_encode(out)], $YUMETE_DUMP)
endfunction

if v:vim_did_enter
  call s:Dump() | qall!
else
  autocmd VimEnter * ++once ++nested call s:Dump() | qall!
endif
