use std::collections::{LinkedList, HashMap};

use serde::Serialize;
use wasm_bindgen::{prelude::wasm_bindgen, JsValue};

use crate::{
    instructions::{RegisterOperand, Size, Label},
    interpreter::Flags,
};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum MutationOperation {
    WriteRegister {
        register: RegisterOperand,
        old: u32,
        size: Size,
    },
    WriteMemory {
        address: usize,
        old: u32,
        size: Size,
    },
    WriteMemoryBytes {
        address: usize,
        old: Box<Vec<u8>>,
    },
    PushCall {
        to: usize,
        from: usize,
    },
    PopCall {
        to: usize,
        from: usize,
    }
}
#[derive(Serialize)]
pub struct ExecutionStep {
    mutations: Vec<MutationOperation>,
    pc: usize,
    ccr: Flags,
}

impl ExecutionStep {
    pub fn new(pc: usize, ccr: Flags) -> Self {
        Self {
            mutations: vec![],
            pc,
            ccr,
        }
    }
    pub fn add_mutation(&mut self, mutation: MutationOperation) {
        self.mutations.push(mutation);
    }
    pub fn set_pc(&mut self, pc: usize) {
        self.pc = pc;
    }
    pub fn set_ccr(&mut self, ccr: Flags) {
        self.ccr = ccr;
    }
    pub fn get_mutations(&self) -> &Vec<MutationOperation> {
        &self.mutations
    }
    pub fn get_pc(&self) -> usize {
        self.pc
    }
    pub fn get_ccr(&self) -> Flags {
        self.ccr
    }
}
#[wasm_bindgen]
pub struct Debugger {
    history: LinkedList<ExecutionStep>,
    history_size: usize,
    call_stack: Vec<usize>,
    labels: HashMap<usize, Label>
}


impl Debugger {
    pub fn new(history_size: usize, labels: &HashMap<String, Label>) -> Self {
        let mut labels_map = HashMap::new();
        for (_, label) in labels {
            labels_map.insert(label.address, label.clone());
        }
        Self {
            history: LinkedList::new(),
            history_size,
            call_stack: vec![],
            labels: labels_map
        }
    }
    pub fn add_step(&mut self, step: ExecutionStep) {
        self.history.push_back(step);
        if self.history.len() > self.history_size {
            self.history.pop_front();
        }
    }
    pub fn pop_step(&mut self) -> Option<ExecutionStep> {
        self.history.pop_back()
    }
    pub fn get_previous_mutations(&self) -> Option<&Vec<MutationOperation>> {
        match self.history.back() {
            Some(step) => Some(step.get_mutations()),
            None => None,
        }
    }
    pub fn can_undo(&self) -> bool {
        self.history.len() > 0
    }
    pub fn get_last_step(&self) -> Option<&ExecutionStep> {
        self.history.back()
    }
    pub fn add_mutation(&mut self, operation: MutationOperation) {
        self.history
            .back_mut()
            .expect("No history to add mutation to")
            .add_mutation(operation);
    }
    pub fn get_history(&self) -> &LinkedList<ExecutionStep> {
        &self.history
    }
    pub fn get_last_steps(&self, count: usize) -> Vec<&ExecutionStep> {
        self.history
            .iter()
            .rev()
            .take(count)
            .collect::<Vec<&ExecutionStep>>()
    }
    pub fn get_labels(&self) -> &HashMap<usize, Label> {
        &self.labels
    }
    pub fn push_call(&mut self, address: usize) {
        self.call_stack.push(address);
    }
    pub fn pop_call(&mut self) -> Option<usize> {
        self.call_stack.pop()
    }
    pub fn to_call_stack(&self) -> Vec<Label> {
        self.call_stack.iter().map(|address| {
            match self.labels.get(address) {
                Some(label) => label.clone(),
                None => Label {
                    name: "".to_string(),
                    address: *address,
                    line: 0
                }
            }
        }).collect()
    }
}
