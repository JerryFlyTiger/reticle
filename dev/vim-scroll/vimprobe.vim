set lines=24 columns=80
set nomore
let g:out = []
function! St(tag)
  redraw
  call add(g:out, printf('%-26s w0=%3d cur=%3d w$=%3d scroll=%d so=%d', a:tag, line('w0'), line('.'), line('w$'), &scroll, &scrolloff))
endfunction
function! Try(tag, keys)
  execute "normal " . a:keys
  call St(a:tag)
endfunction
call St('start')
call Try('C-f #1', "\<C-f>")
call Try('C-f #2', "\<C-f>")
call Try('C-b #1', "\<C-b>")
normal gg
call St('gg')
call Try('C-b at top', "\<C-b>")
normal G
call St('G')
call Try('C-f at end #1', "\<C-f>")
call Try('C-f at end #2', "\<C-f>")
normal gg
call Try('C-e #1 (cur on top)', "\<C-e>")
call Try('C-e #2', "\<C-e>")
call Try('3 C-e', "3\<C-e>")
call Try('C-y #1', "\<C-y>")
normal 10G
call St('10G')
call Try('C-y cur mid', "\<C-y>")
call Try('C-y at top ws1', "\<C-y>")
normal gg
call Try('C-d #1', "\<C-d>")
call Try('C-d #2', "\<C-d>")
call Try('C-u #1', "\<C-u>")
normal gg
call Try('5 C-d', "5\<C-d>")
call Try('C-d after 5C-d', "\<C-d>")
set scroll=0
normal G
call Try('C-d at end', "\<C-d>")
normal 95G
call St('95G')
call Try('C-d near end', "\<C-d>")
normal gg
call Try('C-u at top', "\<C-u>")
normal 3G
call Try('C-u at 3G', "\<C-u>")
normal 50G
call St('50G')
call Try('zz', "zz")
call Try('zt', "zt")
call Try('zb', "zb")
call Try('z<CR>', "z\<CR>")
call Try('z.', "z.")
call Try('z-', "z-")
normal 3G
call Try('zz at 3G', "zz")
call Try('zb at 3G', "zb")
normal 98G
call Try('zt at 98G', "zt")
call Try('zz at 98G', "zz")
normal 50G
call Try('30zt', "30zt")
call Try('20zz', "20zz")
call writefile(g:out, $OUT)
qa!
