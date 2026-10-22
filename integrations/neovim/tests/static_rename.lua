local api=vim.api
local output=assert(vim.env.SKRIPT_RENAME_GUARD_OUTPUT)
local here=debug.getinfo(1,'S').source:gsub('^@','')
package.path=vim.fs.dirname(vim.fs.dirname(here))..'/?.lua;'..package.path
local flushed=0
package.loaded['vim.lsp._changetracking']={flush=function()flushed=flushed+1 end}
local guard=require('skript_rename')
local cap={};guard.capabilities(cap);assert(cap.workspace.workspaceEdit.documentChanges)
local cases={}
local serial=0
local function fixture()
 serial=serial+1;local buffers={}
 for i=1,2 do
  local b=api.nvim_create_buf(true,false)
  api.nvim_buf_set_name(b,vim.fn.tempname()..'-'..serial..'-'..i..'.sk')
  api.nvim_buf_set_lines(b,0,-1,false,{i==1 and 'alpha' or 'beta'})
  vim.lsp.util.buf_versions[b]=0;buffers[i]=b
 end
 local c={attached_buffers={[buffers[1]]=true,[buffers[2]]=true},handlers={},stopped=false,rpc={notify=function()return true end}}
 function c:is_stopped()return self.stopped end
 function c:request(method,params,callback,buffer)self.reply=callback;self.requested=method;return true,serial end
 c.handlers['textDocument/rename']=function(err,edit)
  c.error,c.edit=err,edit
  if not err and edit then vim.lsp.util.apply_workspace_edit(edit,'utf-16')end
 end
 for _,b in ipairs(buffers)do guard.attach(c,b)end
 local edit={documentChanges={}}
 for i,b in ipairs(buffers)do edit.documentChanges[i]={textDocument={uri=vim.uri_from_bufnr(b),version=0},edits={{range={start={line=0,character=0},['end']={line=0,character=1}},newText=i==1 and 'x' or 'y'}}}end
 return c,buffers,edit
end
local function test(name,mutate,expected,dot)
 local c,b,edit=fixture();local before=flushed
 if dot then c.request('textDocument/rename',{},nil,b[1])else c:request('textDocument/rename',{},nil,b[1])end;assert(flushed-before==2)
 if mutate then mutate(c,b,edit)end
 c.reply(nil,edit,{})
 if expected then assert(not c.error,name);assert(api.nvim_buf_get_lines(b[1],0,1,false)[1]=='xlpha');assert(api.nvim_buf_get_lines(b[2],0,1,false)[1]=='yeta')
 else assert(c.error and c.error.code==-32801 and c.edit==nil,name);assert(api.nvim_buf_get_lines(b[1],0,1,false)[1]=='alpha',name..' partially changed first buffer')end
 cases[#cases+1]={name=name,passed=true,error=c.error}
 for _,buffer in ipairs(b)do if api.nvim_buf_is_valid(buffer)then api.nvim_buf_delete(buffer,{force=true})end end
end
local ok,err=xpcall(function()
 test('version zero on every document',nil,true)
 test('dot request applies checked edits',nil,true,true)
 test('dot request rejects stale later target',function(c,b,e)vim.lsp.util.buf_versions[b[2]]=1 end,false,true)
 test('same URI version and tick close-open without attach',function(c,b,e)
  local uri=vim.uri_from_bufnr(b[2]);c.rpc.notify('textDocument/didClose',{textDocument={uri=uri}})
  c.rpc.notify('textDocument/didOpen',{textDocument={uri=uri,version=0}})
 end)
 test('unrelated document open invalidates pending rename',function(c,b,e)c.rpc.notify('textDocument/didOpen',{textDocument={uri='file:///unrelated.sk',version=0}})end)
 test('non-lifecycle notifications preserve pending rename',function(c,b,e)assert(c.rpc.notify('workspace/didChangeConfiguration',{}))end,true)
 test('second target version changed',function(c,b,e)vim.lsp.util.buf_versions[b[2]]=1 end)
 test('first target version changed',function(c,b,e)vim.lsp.util.buf_versions[b[1]]=1 end)
 test('second target text changed without flushed version',function(c,b,e)api.nvim_buf_set_lines(b[2],0,-1,false,{'user edit'})end)
 test('future server version',function(c,b,e)e.documentChanges[2].textDocument.version=1 end)
 test('null version',function(c,b,e)e.documentChanges[2].textDocument.version=vim.NIL end)
 test('reattach same buffer version and text',function(c,b,e)guard.attach(c,b[2])end)
 test('detached target',function(c,b,e)c.attached_buffers[b[2]]=nil end)
 test('closed buffer',function(c,b,e)api.nvim_buf_delete(b[2],{force=true})end)
 test('replaced URI',function(c,b,e)api.nvim_buf_set_name(b[2],api.nvim_buf_get_name(b[2])..'-moved')end)
 test('unmodifiable later target',function(c,b,e)vim.bo[b[2]].modifiable=false end)
 test('stopped client',function(c,b,e)c.stopped=true end)
 test('versionless changes rejected',function(c,b,e)e.documentChanges=nil;e.changes={}end)
 test('resource operations rejected',function(c,b,e)e.documentChanges[2]={kind='delete',uri=vim.uri_from_bufnr(b[2])}end)
 local c,b,edit=fixture();c:request('textDocument/hover',{},function()end,b[1]);assert(c.requested=='textDocument/hover')
 c:request('textDocument/rename',{},nil,b[1]);c.reply({code=-32800,message='cancelled'},nil,{});assert(c.error.code==-32800 and not c.edit)
 for _,buffer in ipairs(b)do api.nvim_buf_delete(buffer,{force=true})end
 cases[#cases+1]={name='non-rename and cancellation pass through',passed=true}
end,debug.traceback)
local file=assert(io.open(output,'wb'));file:write(vim.json.encode({status=ok and 'passed' or 'failed',error=err,cases=cases,privateInputsOpened=false,mainConfigLoaded=false}),'\n');file:close()
if not ok then error(err)end
vim.cmd('qa!')
