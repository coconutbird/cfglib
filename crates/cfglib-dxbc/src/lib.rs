//! SM4/SM5 shader bytecode adapter for `cfglib`.
//!
//! Provides a [`FlowControl`] implementation for [`dxbc`] shader
//! instructions, enabling automatic CFG construction from decoded
//! shader programs.
//!
//! # Example
//!
//! ```ignore
//! let program = /* decoded dxbc::shex::ir::Program */;
//! let cfg = cfglib_dxbc::build_cfg(&program);
//! println!("{}", cfg.to_dot());
//! ```

extern crate alloc;
use alloc::borrow::Cow;

use dxbc::shex::{Instruction, Opcode, Program};
use cfglib::{BuildError, Cfg, CfgBuilder, FlowControl, FlowEffect};

/// Newtype wrapper around a `dxbc` instruction to satisfy the orphan rule.
#[derive(Debug, Clone)]
pub struct Sm4Instruction(pub Instruction);

impl FlowControl for Sm4Instruction {
    fn flow_effect(&self) -> FlowEffect {
        match self.0.opcode {
            // Structured conditional regions.
            Opcode::If => FlowEffect::ConditionalOpen,
            Opcode::Else => FlowEffect::ConditionalAlternate,
            Opcode::EndIf => FlowEffect::ConditionalClose,

            // Switch/case — break inside a switch exits the switch.
            Opcode::Switch => FlowEffect::SwitchOpen,
            Opcode::Case => FlowEffect::SwitchCase,
            Opcode::Default => FlowEffect::SwitchCase,
            Opcode::EndSwitch => FlowEffect::SwitchClose,

            // Structured loops.
            Opcode::Loop => FlowEffect::LoopOpen,
            Opcode::EndLoop => FlowEffect::LoopClose,

            // Break / continue.
            Opcode::Break => FlowEffect::Break,
            Opcode::Breakc => FlowEffect::ConditionalBreak,
            Opcode::Continue => FlowEffect::Continue,
            Opcode::Continuec => FlowEffect::ConditionalContinue,

            // Returns.
            Opcode::Ret => FlowEffect::Return,
            Opcode::Retc => FlowEffect::ConditionalReturn,

            // Calls.
            Opcode::Call | Opcode::InterfaceCall => FlowEffect::Call,
            Opcode::Callc => FlowEffect::ConditionalCall,

            // Discard marks the pixel for discard but execution continues.
            Opcode::Discard => FlowEffect::Fallthrough,

            // Actual terminators.
            Opcode::Abort => FlowEffect::Terminate,

            // Labels.
            Opcode::Label => FlowEffect::Label,

            // Declarations.
            op if is_declaration(op) => FlowEffect::Declaration,

            // Everything else is a normal ALU / memory instruction.
            _ => FlowEffect::Fallthrough,
        }
    }

    fn display_mnemonic(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0.opcode.name())
    }
}

/// Returns `true` if the opcode is a declaration (not executable code).
fn is_declaration(op: Opcode) -> bool {
    matches!(
        op,
        Opcode::DclGlobalFlags
            | Opcode::DclInput
            | Opcode::DclInputSgv
            | Opcode::DclInputSiv
            | Opcode::DclInputPs
            | Opcode::DclInputPsSgv
            | Opcode::DclInputPsSiv
            | Opcode::DclOutput
            | Opcode::DclOutputSgv
            | Opcode::DclOutputSiv
            | Opcode::DclResource
            | Opcode::DclSampler
            | Opcode::DclConstantBuffer
            | Opcode::DclTemps
            | Opcode::DclIndexableTemp
            | Opcode::DclIndexRange
            | Opcode::DclGsInputPrimitive
            | Opcode::DclGsOutputPrimitiveTopology
            | Opcode::DclMaxOutputVertexCount
            | Opcode::DclGsInstanceCount
            | Opcode::DclOutputControlPointCount
            | Opcode::DclInputControlPointCount
            | Opcode::DclTessDomain
            | Opcode::DclTessPartitioning
            | Opcode::DclTessOutputPrimitive
            | Opcode::DclHsMaxTessFactor
            | Opcode::DclHsForkPhaseInstanceCount
            | Opcode::DclHsJoinPhaseInstanceCount
            | Opcode::DclThreadGroup
            | Opcode::DclUnorderedAccessViewTyped
            | Opcode::DclUnorderedAccessViewRaw
            | Opcode::DclUnorderedAccessViewStructured
            | Opcode::DclThreadGroupSharedMemoryRaw
            | Opcode::DclThreadGroupSharedMemoryStructured
            | Opcode::DclResourceRaw
            | Opcode::DclResourceStructured
            | Opcode::DclStream
            | Opcode::DclFunctionBody
            | Opcode::DclFunctionTable
            | Opcode::DclInterface
            | Opcode::HsDecls
            | Opcode::HsControlPointPhase
            | Opcode::HsForkPhase
            | Opcode::HsJoinPhase
            | Opcode::CustomData
    )
}

/// Build a control-flow graph from a decoded shader program.
///
/// Returns an error if the shader contains mismatched structured
/// control-flow instructions (e.g. `else` without `if`).
pub fn build_cfg(program: &Program) -> Result<Cfg<Sm4Instruction>, BuildError> {
    CfgBuilder::build(program.instructions.iter().cloned().map(Sm4Instruction))
}
