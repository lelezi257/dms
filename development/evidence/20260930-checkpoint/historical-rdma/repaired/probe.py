import os,sys,hashlib,json,time
op,path=sys.argv[1:];data=(bytes(range(251))*((4194321//251)+1))[:4194321]
t=time.monotonic()
if op=='write':
 fd=os.open(path,os.O_CREAT|os.O_TRUNC|os.O_RDWR,0o600);written=0
 while written<len(data):written+=os.write(fd,data[written:])
 os.fsync(fd);os.close(fd)
 result={'write':written,'fsync':'SUCCESS','close':'SUCCESS'}
else:
 with open(path,'rb') as f:actual=f.read()
 assert actual==data,(len(actual),hashlib.sha256(actual).hexdigest())
 result={'read':len(actual),'exact_bytes':'PASS'}
result.update(sha256=hashlib.sha256(data).hexdigest(),elapsed_s=time.monotonic()-t)
print(json.dumps(result))
