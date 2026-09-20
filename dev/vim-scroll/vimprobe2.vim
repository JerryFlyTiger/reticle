set lines=24 columns=80
set nomore
let g:out = []
function! St(tag)
  redraw
  call add(g:out, printf('%-26s w0=%3d cur=%3d col=%2d w$=%3d', a:tag, line('w0'), line('.'), col('.'), line('w$')))
endfunction
function! Try(tag, keys)
  execute "normal " . a:keys
  call St(a:tag)
endfunction
normal 23G
call St('23G (last visible)')
call Try('C-f cur on last', "\<C-f>")
normal 22G
call St('22G (top of page 2)')
call Try('C-b cur on top', "\<C-b>")
normal 22G
normal 2G
call St('2G')
call Try('C-b cur 2 w0 1', "\<C-b>")
normal 40G
call St('40G')
call Try('C-e cur mid', "\<C-e>")
normal 50G
normal 0
call St('50G col 1')
call Try('z<CR> indented', "z\<CR>")
normal 51G
normal 0
call Try('z. unindented', "z.")
normal 50G
normal 0
call Try('zt indented (col?)', "zt")
normal 50G
normal $
call Try('z- from $', "z-")
normal 50G
call Try('3 C-d (scroll=3)', "3\<C-d>")
call Try('C-y after (cur?)', "\<C-y>")
call Try('j after 3C-d', "j")
normal 99G
call Try('C-f at 99', "\<C-f>")
normal 90G
call Try('C-f at 90 (w0?)', "\<C-f>")
normal 80G
call St('80G')
call Try('C-f at 80', "\<C-f>")
normal 10G
call St('10G w0?')
call Try('C-e x5', "5\<C-e>")
call Try('C-e cur leaves', "5\<C-e>")
call writefile(g:out, $OUT)
qa!
