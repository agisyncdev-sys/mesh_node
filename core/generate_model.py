import onnx
from onnx import helper, TensorProto

# Dynamic batch axis ('N') so both [1,1] and [4,1] inputs work at runtime.
node_def = helper.make_node('Identity', ['input'], ['output'])

graph_def = helper.make_graph(
    [node_def],
    'minimal-model',
    [helper.make_tensor_value_info('input',  TensorProto.FLOAT, ['N', 1])],
    [helper.make_tensor_value_info('output', TensorProto.FLOAT, ['N', 1])]
)

# opset 13 is stable and well-supported. IR version 7 is explicitly capped to
# remain compatible with ONNX Runtime 1.17.1 (max supported IR version = 9).
opset_imports = [helper.make_opsetid('', 13)]
model_def = helper.make_model(graph_def, producer_name='mesh-core', opset_imports=opset_imports)
model_def.ir_version = 7

onnx.checker.check_model(model_def)
onnx.save(model_def, 'minimal.onnx')
print(f'Saved minimal.onnx  IR version={model_def.ir_version}  opset={model_def.opset_import[0].version}')
