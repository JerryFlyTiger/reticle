" vimprobe3.vim -- the two boundary rules M139's implementer corrected after
" measuring vim itself, re-measured so the record's claim has an artifact:
"   C-f near the end: from w0 60/70/78/79/80 (h=23, 100 lines)
"   C-d near the end: from cursor 70/80/85/90 with the default 'scroll'
" OUT=vimprobe3.out script -q /dev/null vim -u NONE -N -S vimprobe3.vim hundred.txt
set lines=24 columns=80
set nomore
let g:out = []
function! St(tag)
  redraw
  call add(g:out, printf('%-22s w0=%3d cur=%3d w$=%3d scroll=%d', a:tag, line('w0'), line('.'), line('w$'), &scroll))
endfunction
for w in [60, 70, 78, 79, 80]
  execute "normal " . w . "Gzt"
  call St('w0 ' . w)
  execute "normal \<C-f>"
  call St('  C-f from ' . w)
endfor
for c in [70, 80, 85, 90]
  normal gg
  set scroll=0
  execute "normal " . c . "G"
  call St('cur ' . c)
  execute "normal \<C-d>"
  call St('  C-d from ' . c)
endfor
call writefile(g:out, $OUT)
qa!
