use std::{collections::HashSet, fmt::Display};

use log::error;

use crate::{
    renderer_spec::RendererId,
    scene::{
        constraints::input_count::InputCountConstraint, transition::TransitionSpec, NodeId,
        NodeParams, OutputId,
    },
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SceneSpecValidationError {
    #[error("Unknown node \"{missing_node}\" used as an input in the node \"{node}\". Node is not defined in the scene and it was not registered as an input.")]
    UnknownInputPadOnNode { missing_node: NodeId, node: NodeId },
    #[error("Unknown node \"{missing_node}\" is connected to the output stream \"{output}\".")]
    UnknownInputPadOnOutput {
        missing_node: NodeId,
        output: OutputId,
    },
    #[error(
        "Unknown output stream \"{0}\". Register it first before using it in the scene definition."
    )]
    UnknownOutput(NodeId),
    #[error("Invalid node id. There is more than one node with the \"{0}\" id.")]
    DuplicateNodeNames(NodeId),
    #[error("Invalid node id. There is already an input stream with the \"{0}\" id.")]
    DuplicateNodeAndInputNames(NodeId),
    #[error("Cycles between nodes are not allowed. Node \"{0}\" depends on itself via input_pads or fallback option.")]
    CycleDetected(NodeId),
    #[error(transparent)]
    UnusedNodes(#[from] UnusedNodesError),
    #[error("Invalid params for node \"{1}\".")]
    InvalidNodeSpec(#[source] NodeSpecValidationError, NodeId),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub struct UnusedNodesError(pub HashSet<NodeId>);

impl Display for UnusedNodesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut unused_nodes: Vec<String> = self.0.iter().map(ToString::to_string).collect();
        unused_nodes.sort();
        write!(
            f,
            "There are unused nodes in the scene definition: {0}",
            unused_nodes.join(", ")
        )
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UnsatisfiedConstraintsError {
    #[error(transparent)]
    InvalidInputsCount(InputCountConstraintValidationError),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub struct InputCountConstraintValidationError {
    pub node_identifier: NodeIdentifier,
    pub input_count_constrain: InputCountConstraint,
    pub defined_input_pad_count: u32,
}

impl Display for InputCountConstraintValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let expects = match self.input_count_constrain {
            InputCountConstraint::Exact { fixed_count: 0 } => {
                "does not excepts input pads".to_owned()
            }
            InputCountConstraint::Exact { fixed_count: 1 } => {
                "expects exactly one input pad".to_owned()
            }
            InputCountConstraint::Exact { fixed_count } => {
                format!("expects exactly {fixed_count} input pads")
            }
            InputCountConstraint::Range {
                lower_bound,
                upper_bound,
            } => format!("expects at least {lower_bound} and at most {upper_bound} input pads"),
        };

        let specified = match self.defined_input_pad_count {
            0 => "none input pads were specified.".to_owned(),
            1 => "one input pad was specified.".to_owned(),
            n => format!("{n} input pads were specified."),
        };

        write!(f, "{} {}, but {}", self.node_identifier, expects, specified)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NodeSpecValidationError {
    #[error(transparent)]
    Builtin(#[from] BuiltinSpecValidationError),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BuiltinSpecValidationError {
    #[error("Transformation \"fixed_position_layout\" expects {input_count} texture layouts (the same as number of input pads), but {layout_count} layouts were specified.")]
    FixedLayoutInvalidLayoutCount { layout_count: u32, input_count: u32 },
    #[error("Each entry in texture_layouts in transformation \"fixed_position_layout\" requires either bottom or top coordinate.")]
    FixedLayoutTopBottomRequired,
    #[error("Each entry in texture_layouts in transformation \"fixed_position_layout\" requires either right or left coordinate.")]
    FixedLayoutLeftRightRequired,
    #[error("Fields \"top\" and \"bottom\" are mutually exclusive, you can only specify one in texture layout in \"fixed_position_layout\" transformation.")]
    FixedLayoutTopBottomOnlyOne,
    #[error("Fields \"left\" and \"right\" are mutually exclusive, you can only specify one in texture layout in \"fixed_position_layout\" transformation.")]
    FixedLayoutLeftRightOnlyOne,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NodeIdentifier {
    WebRenderer(RendererId),
    Shader(RendererId),
    Text,
    Image(RendererId),
    Builtin(&'static str),
    Transition(&'static str, &'static str),
}

impl From<&NodeParams> for NodeIdentifier {
    fn from(node_params: &NodeParams) -> Self {
        match node_params {
            NodeParams::WebRenderer { instance_id } => Self::WebRenderer(instance_id.clone()),
            NodeParams::Shader { shader_id, .. } => Self::Shader(shader_id.clone()),
            NodeParams::Text(_) => Self::Text,
            NodeParams::Image { image_id } => Self::Image(image_id.clone()),
            NodeParams::Builtin(transformation) => {
                Self::Builtin(transformation.transformation_name())
            }
            NodeParams::Transition(TransitionSpec { start, end, .. }) => {
                Self::Transition(start.transformation_name(), end.transformation_name())
            }
        }
    }
}

impl Display for NodeIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeIdentifier::WebRenderer(instance_id) => {
                write!(f, "\"{}\" web renderer", instance_id)
            }
            NodeIdentifier::Shader(shader_id) => write!(f, "\"{}\" shader", shader_id),
            NodeIdentifier::Text => write!(f, "Text"),
            NodeIdentifier::Image(image_id) => write!(f, "\"{}\" image", image_id),
            NodeIdentifier::Builtin(builtin_name) => {
                write!(f, "\"{}\" builtin transformation", builtin_name)
            }
            NodeIdentifier::Transition(_, end) => {
                // end state of a transition is a source of constraints
                write!(f, "\"{}\" builtin transformation", end)
            }
        }
    }
}

pub struct ErrorStack<'a>(Option<&'a (dyn std::error::Error + 'static)>);

impl<'a> ErrorStack<'a> {
    pub fn new(value: &'a (dyn std::error::Error + 'static)) -> Self {
        ErrorStack(Some(value))
    }

    pub fn into_string(self) -> String {
        let stack: Vec<String> = self.map(ToString::to_string).collect();
        stack.join("\n")
    }
}

impl<'a> Iterator for ErrorStack<'a> {
    type Item = &'a (dyn std::error::Error + 'static);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.map(|err| {
            self.0 = err.source();
            err
        })
    }
}
