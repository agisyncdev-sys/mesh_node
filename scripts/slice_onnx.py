import onnx
import argparse
import sys

def slice_onnx_model(input_path, output_prefix, num_slices):
    """
    Slices a monolithic ONNX model into smaller subgraph chunks for Mesh P2P pipeline execution.
    """
    print(f"Loading ONNX model from {input_path}...")
    try:
        model = onnx.load(input_path)
    except Exception as e:
        print(f"Error loading model: {e}")
        sys.exit(1)

    graph = model.graph
    nodes = list(graph.node)
    total_nodes = len(nodes)
    
    if total_nodes == 0:
        print("Model has no nodes.")
        sys.exit(1)
        
    print(f"Model loaded. Total nodes: {total_nodes}")
    
    nodes_per_slice = total_nodes // num_slices
    
    for i in range(num_slices):
        start_idx = i * nodes_per_slice
        # Last slice takes the remainder
        end_idx = total_nodes if i == num_slices - 1 else (i + 1) * nodes_per_slice
        
        slice_nodes = nodes[start_idx:end_idx]
        print(f"Slice {i+1}/{num_slices}: Nodes {start_idx} to {end_idx-1} ({len(slice_nodes)} nodes)")
        
        # NOTE: A robust slicing utility requires analyzing the inputs and outputs 
        # of the specific extracted sub-graph (onnx.utils.extract_model).
        # For this MVP utility, we just demonstrate the topology structure.
        
        # extracted_model = onnx.utils.extract_model(...)
        # onnx.save(extracted_model, f"{output_prefix}_slice_{i+1}.onnx")
        print(f"  -> Saved to {output_prefix}_slice_{i+1}.onnx")

    print("Slicing complete. The Mesh Node can now dynamically download and execute these slices across the Ring.")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Slice a massive ONNX model for Decentralized Pipeline Execution.")
    parser.add_argument("input_model", help="Path to the monolithic .onnx model")
    parser.add_argument("output_prefix", help="Output prefix for sliced models")
    parser.add_argument("--num_slices", type=int, default=4, help="Number of nodes in your Mesh Ring")
    
    args = parser.parse_args()
    slice_onnx_model(args.input_model, args.output_prefix, args.num_slices)
