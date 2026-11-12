#!/usr/bin/env python3
"""Warm-cache CLI wall time and per-child peak RSS; never invokes a shell."""
import argparse, hashlib, json, os, platform, shutil, statistics, subprocess, sys, tempfile, time
from pathlib import Path
ROOT=Path(__file__).resolve().parent
EXPECTED={"startup":"", "variables":"20000\n", "calls":"10000\n", "loops":"22500\n", "strings":"96000\n", "lists":"3033744\n", "maps":"512000\n", "files":"8388608\n", "processes":"20\n", "memory":"49\n"}

def measure(command):
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
        started=time.perf_counter_ns()
        child=subprocess.Popen(command,stdout=output,stderr=errors,env={**os.environ,"LC_ALL":"C","PYTHONHASHSEED":"0"})
        usage=None
        if hasattr(os,"wait4"):
            _,status,usage=os.wait4(child.pid,0)
            child.returncode=os.waitstatus_to_exitcode(status)
        else: child.wait()
        elapsed=(time.perf_counter_ns()-started)/1e6
        output.seek(0); errors.seek(0)
        stdout=output.read().decode("utf-8"); stderr=errors.read().decode("utf-8",errors="replace")
        if child.returncode: raise RuntimeError(f"{command}: exit {child.returncode}: {stderr[:1000]}")
        rss=None if usage is None else usage.ru_maxrss/(1024*1024 if sys.platform=="darwin" else 1024)
        return {"wall_ms":elapsed,"peak_rss_mib":rss},stdout

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scr",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    parser.add_argument("--reference",type=Path,help="optional pre-change binary measured in the same run")
    parser.add_argument("--samples",type=int,default=11)
    parser.add_argument("--warmups",type=int,default=2)
    args=parser.parse_args()
    if args.samples<1 or args.warmups<0: parser.error("samples must be positive and warmups nonnegative")
    binary=args.scr.resolve(); bash=shutil.which("bash"); true=shutil.which("true")
    bash_version=subprocess.check_output([bash,"--version"],text=True).splitlines()[0] if bash else None
    bash_maps=bash_version is not None and int(bash_version.split("version ")[1].split(".")[0])>=4
    reference=args.reference.resolve() if args.reference else None
    report={"platform":platform.platform(),"machine":platform.machine(),"logical_cpus":os.cpu_count(),"python":sys.version,"bash":bash_version,"scr":str(binary),"binary_bytes":binary.stat().st_size,"binary_sha256":hashlib.sha256(binary.read_bytes()).hexdigest(),"samples":args.samples,"warmups":args.warmups,"rss_method":"wait4 ru_maxrss per child; includes descendants where OS accounts them" if hasattr(os,"wait4") else "unavailable", "results":[],"unavailable":[],"fixture_sha256":{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in (ROOT/"fixtures").iterdir() if p.is_file()}}
    if reference: report["reference"]={"path":str(reference),"binary_bytes":reference.stat().st_size,"binary_sha256":hashlib.sha256(reference.read_bytes()).hexdigest()}
    with tempfile.TemporaryDirectory(prefix="scripta-benchmark-") as directory:
        directory=Path(directory); text=directory/"input.txt"; text.write_bytes(b"x"*(1024*1024)); data=directory/"values.json"; data.write_text(json.dumps(list(range(4096))))
        for name,expected in EXPECTED.items():
            extra=[str(text)] if name=="files" else [str(data)] if name=="memory" else [true] if name=="processes" else []
            languages=([("scripta_baseline",[str(reference)])] if reference else [])+[("scripta",[str(binary)]),("bash",[bash] if bash else []),("python",[sys.executable])]
            for language,command in languages:
                extension="scr" if language.startswith("scripta") else {"bash":"sh","python":"py"}[language]; fixture=ROOT/"fixtures"/f"{name}.{extension}"
                if not command or not fixture.exists() or name=="maps" and language=="bash" and not bash_maps or name=="processes" and not true:
                    report["unavailable"].append({"workload":name,"language":language,"reason":"missing executable/fixture, or Bash <4 has no associative arrays"}); continue
                invocation=command+[str(fixture)]+extra
                observations=[]
                for index in range(args.warmups+args.samples):
                    sample,stdout=measure(invocation)
                    if stdout!=expected: raise RuntimeError(f"wrong result for {language}/{name}: {stdout!r}, expected {expected!r}")
                    if index>=args.warmups: observations.append(sample)
                times=[v["wall_ms"] for v in observations]; peaks=[v["peak_rss_mib"] for v in observations if v["peak_rss_mib"] is not None]
                result={"workload":name,"language":language,"median_ms":statistics.median(times),"min_ms":min(times),"max_ms":max(times),"peak_rss_mib":max(peaks) if peaks else None,"observations":observations}
                report["results"].append(result)
                print(f"{name:10} {language:7} {result['median_ms']:9.3f} ms {str(result['peak_rss_mib']):>8} MiB",flush=True)
    args.output.parent.mkdir(parents=True,exist_ok=True); args.output.write_text(json.dumps(report,indent=2)+"\n")
if __name__=="__main__": main()
