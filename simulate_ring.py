import subprocess
import time
import sys
import os

def main():
    print("=== STARTING P2P RING SIMULATION ===")
    
    # Use absolute path to the compiled executable
    runner_path = os.path.abspath(os.path.join("core", "target", "debug", "node_runner.exe"))
    print(f"Runner binary path: {runner_path}")
    
    # Start Node-C (Port 50063 -> Node-A on 50061)
    print("Launching Node-C (Port 50063 -> 50061)...")
    proc_c = subprocess.Popen(
        [runner_path, "Node-C", "50063", "50061", "3"],
        cwd="core",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True
    )
    
    # Start Node-B (Port 50062 -> Node-C on 50063)
    print("Launching Node-B (Port 50062 -> 50063)...")
    proc_b = subprocess.Popen(
        [runner_path, "Node-B", "50062", "50063", "3"],
        cwd="core",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True
    )
    
    # Give B and C a moment to bind to their ports
    time.sleep(2)
    
    # Start Node-A (Port 50061 -> Node-B on 50062) and trigger prompt
    print("Launching Node-A with prompt 'Hello from the Ring Mesh!' (Port 50061 -> 50062)...")
    proc_a = subprocess.Popen(
        [runner_path, "Node-A", "50061", "50062", "3", "Hello from the Ring Mesh!"],
        cwd="core",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True
    )
    
    stdout_a, stderr_a = "", ""
    try:
        # Wait for Node-A to complete and print the result
        stdout_a, stderr_a = proc_a.communicate(timeout=15)
        print("\n--- Node-A Output ---")
        print(stdout_a)
        if stderr_a:
            print("--- Node-A Errors ---")
            print(stderr_a)
    except subprocess.TimeoutExpired as e:
        print("\n!!! ERROR: Node-A timed out waiting for ring completion !!!")
        proc_a.kill()
        stdout_a, stderr_a = proc_a.communicate()
        print("--- Node-A Output (Before Kill) ---")
        print(stdout_a)
        print("--- Node-A Errors (Before Kill) ---")
        print(stderr_a)
    finally:
        # Clean up processes
        proc_b.terminate()
        proc_c.terminate()
        stdout_b, _ = proc_b.communicate()
        stdout_c, _ = proc_c.communicate()
        print("--- Node-B Output ---")
        print(stdout_b)
        print("--- Node-C Output ---")
        print(stdout_c)
    
    if "SUCCESS: Ring All-Reduce completed!" in stdout_a:
        print("=== SIMULATION PASSED SUCCESSFULLY ===")
        sys.exit(0)
    else:
        print("=== SIMULATION FAILED ===")
        sys.exit(1)

if __name__ == "__main__":
    main()
